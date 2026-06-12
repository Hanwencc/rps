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
};
use tracing::{debug, error, info, warn};

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

    pub async fn add(&self, rx_bytes: u64, tx_bytes: u64) {
        if let Err(err) = self
            .state
            .db
            .add_traffic(&self.client_id, &self.tunnel_id, rx_bytes, tx_bytes)
            .await
        {
            debug!(error = %err, "failed to record traffic");
        }
    }
}

pub async fn run(state: AppState, tunnel: TunnelConfig) {
    if let Err(err) = run_inner(state, tunnel).await {
        error!(error = %err, "tcp proxy stopped");
    }
}

async fn run_inner(state: AppState, tunnel: TunnelConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&tunnel.listen).await?;
    info!(tunnel_id = %tunnel.id, listen = %tunnel.listen, "tcp proxy listening");
    loop {
        let (socket, remote_addr) = listener.accept().await?;
        let state = state.clone();
        let tunnel = tunnel.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_tcp(state, tunnel, socket, remote_addr.to_string(), None).await
            {
                warn!(error = %err, "tcp proxy connection failed");
            }
        });
    }
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
    if let Err(err) = state
        .db
        .record_stream_open(
            &route.client_id,
            &route.tunnel_id,
            &request.protocol,
            &request.target,
            &request.remote_addr,
        )
        .await
    {
        debug!(error = %err, "failed to record stream session");
    }
    Ok(stream)
}

pub async fn pipe_tcp_mux(
    socket: TcpStream,
    stream: MuxStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
) -> anyhow::Result<()> {
    let (mut tcp_read, mut tcp_write) = socket.into_split();
    let (mux_write, mut mux_read) = stream.split();

    if let Some(data) = initial_data {
        if let Some(recorder) = &recorder {
            recorder.add(0, data.len() as u64).await;
        }
        mux_write.send_data(data).await?;
    }

    let uplink = {
        let mux_write = mux_write.clone();
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let mut buf = vec![0_u8; 16 * 1024];
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
                    recorder.add(0, n as u64).await;
                }
            }
            anyhow::Ok(())
        })
    };

    let recorder = recorder.clone();
    let downlink = tokio::spawn(async move {
        while let Some(data) = mux_read.recv_data().await {
            tcp_write.write_all(&data).await?;
            if let Some(recorder) = &recorder {
                recorder.add(data.len() as u64, 0).await;
            }
        }
        anyhow::Ok(())
    });

    let _ = tokio::try_join!(uplink, downlink)?;
    Ok(())
}
