use crate::AppState;
use bytes::Bytes;
use rps_core::{
    config::TunnelConfig,
    protocol::{OpenRequest, TargetProtocol},
};
use rps_mux::MuxStream;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};
use tracing::{debug, info, warn};

const TCP_COPY_BUF_SIZE: usize = 64 * 1024;

#[derive(Clone)]
pub struct StreamRoute {
    pub tunnel_id: String,
    pub client_id: String,
}

#[derive(Clone)]
pub struct TrafficRecorder {
    state: AppState,
    tunnel_id: String,
    client_id: String,
}

impl From<&TunnelConfig> for StreamRoute {
    fn from(tunnel: &TunnelConfig) -> Self {
        Self {
            tunnel_id: tunnel.id.clone(),
            client_id: tunnel.client_id.clone(),
        }
    }
}

impl TrafficRecorder {
    pub fn new(state: &AppState, route: &StreamRoute) -> Self {
        Self {
            state: state.clone(),
            tunnel_id: route.tunnel_id.clone(),
            client_id: route.client_id.clone(),
        }
    }

    pub fn add(&self, rx_bytes: u64, tx_bytes: u64) {
        self.state
            .traffic
            .record(&self.client_id, &self.tunnel_id, rx_bytes, tx_bytes);
    }
}

pub async fn serve(
    state: AppState,
    tunnel: TunnelConfig,
    listener: TcpListener,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    run_accept_loop(state, tunnel, listener, shutdown).await
}

async fn run_accept_loop(
    state: AppState,
    tunnel: TunnelConfig,
    listener: TcpListener,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    info!(tunnel_id = %tunnel.id, listen = %tunnel.listen, "tcp proxy listening");
    loop {
        let (socket, remote_addr) = tokio::select! {
            result = listener.accept() => result?,
            _ = shutdown.changed() => {
                info!(tunnel_id = %tunnel.id, listen = %tunnel.listen, "tcp proxy stopping");
                break;
            }
        };
        let state = state.clone();
        let tunnel = tunnel.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_tcp(state, tunnel, socket, remote_addr.to_string(), None).await
            {
                warn!(error = %err, "tcp proxy connection failed");
            }
        });
    }
    Ok(())
}

pub async fn handle_tcp(
    state: AppState,
    tunnel: TunnelConfig,
    socket: TcpStream,
    remote_addr: String,
    initial_data: Option<Bytes>,
) -> anyhow::Result<()> {
    let target = tunnel
        .target
        .clone()
        .ok_or_else(|| anyhow::anyhow!("tcp tunnel target is required"))?;
    let route = StreamRoute::from(&tunnel);
    let recorder = TrafficRecorder::new(&state, &route);
    let stream = open_stream(state, &route, TargetProtocol::Tcp, target, remote_addr).await?;
    pipe_tcp_mux(socket, stream, initial_data, Some(recorder)).await
}

pub async fn open_stream(
    state: AppState,
    route: &StreamRoute,
    protocol: TargetProtocol,
    target: String,
    remote_addr: String,
) -> anyhow::Result<MuxStream> {
    let mux = state
        .clients
        .get(&route.client_id)
        .ok_or_else(|| anyhow::anyhow!("client {} is offline", route.client_id))?
        .clone();
    let request = OpenRequest {
        tunnel_id: route.tunnel_id.clone(),
        protocol,
        target,
        remote_addr,
        timeout_ms: 5000,
    };
    let payload = serde_json::to_vec(&request)?;
    let stream = mux.open_stream(Bytes::from(payload)).await?;
    let db = state.db.clone();
    let client_id = route.client_id.clone();
    let tunnel_id = route.tunnel_id.clone();
    tokio::spawn(async move {
        if let Err(err) = db
            .record_stream_open(
                &client_id,
                &tunnel_id,
                &request.protocol,
                &request.target,
                &request.remote_addr,
            )
            .await
        {
            debug!(error = %err, "failed to record stream session");
        }
    });
    Ok(stream)
}

pub async fn pipe_tcp_mux(
    socket: TcpStream,
    stream: MuxStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
) -> anyhow::Result<()> {
    pipe_tcp_mux_inner(socket, stream, initial_data, recorder, None).await
}

pub async fn pipe_tcp_mux_with_shutdown(
    socket: TcpStream,
    stream: MuxStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    pipe_tcp_mux_inner(socket, stream, initial_data, recorder, Some(shutdown)).await
}

async fn pipe_tcp_mux_inner(
    socket: TcpStream,
    stream: MuxStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
    shutdown: Option<watch::Receiver<bool>>,
) -> anyhow::Result<()> {
    socket.set_nodelay(true)?;
    let (mut tcp_read, mut tcp_write) = socket.into_split();
    let (mux_write, mut mux_read) = stream.split();

    if let Some(data) = initial_data {
        if let Some(recorder) = &recorder {
            recorder.add(0, data.len() as u64);
        }
        mux_write.send_data(data).await?;
    }

    let mut uplink = {
        let mux_write = mux_write.clone();
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let mut buf = vec![0_u8; TCP_COPY_BUF_SIZE];
            loop {
                let n = tcp_read.read(&mut buf).await?;
                if n == 0 {
                    mux_write.close().await?;
                    break;
                }
                mux_write
                    .send_data(Bytes::copy_from_slice(&buf[..n]))
                    .await?;
                if let Some(recorder) = &recorder {
                    recorder.add(0, n as u64);
                }
            }
            anyhow::Ok(())
        })
    };

    let recorder = recorder.clone();
    let mut downlink = tokio::spawn(async move {
        while let Some(data) = mux_read.recv_data().await {
            tcp_write.write_all(&data).await?;
            if let Some(recorder) = &recorder {
                recorder.add(data.len() as u64, 0);
            }
        }
        anyhow::Ok(())
    });

    if let Some(mut shutdown) = shutdown {
        tokio::select! {
            result = &mut uplink => {
                finish_pipe_task(result)?;
                downlink.abort();
                let _ = downlink.await;
            }
            result = &mut downlink => {
                finish_pipe_task(result)?;
                uplink.abort();
                let _ = uplink.await;
            }
            _ = shutdown.changed() => {
                let _ = mux_write.close().await;
                uplink.abort();
                downlink.abort();
                let _ = uplink.await;
                let _ = downlink.await;
            }
        }
    } else {
        let _ = tokio::try_join!(uplink, downlink)?;
    }
    Ok(())
}

fn finish_pipe_task(
    result: Result<anyhow::Result<()>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    result??;
    Ok(())
}
