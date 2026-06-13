use crate::AppState;
use bytes::Bytes;
use rps_core::{
    config::TunnelConfig,
    protocol::{OpenRequest, OpenResponse, TargetProtocol, read_json, write_json},
};
use rps_mux::MuxStream;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    net::{TcpListener, TcpStream},
    sync::watch,
};
use tracing::{debug, info, warn};

const POOL_COPY_BUF_SIZE: usize = 128 * 1024;
const POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_ACQUIRE_RETRIES: usize = 4;

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
            .policy
            .record_route_usage(&self.tunnel_id, rx_bytes, tx_bytes);
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
    let stream = open_pool_stream(state, &route, target, remote_addr).await?;
    if let Some(session_guard) = recorder
        .state
        .tunnel_manager
        .register_session(&route.tunnel_id)
        .await
    {
        let shutdown = session_guard.shutdown_rx();
        let result =
            pipe_pool_with_shutdown(socket, stream, initial_data, Some(recorder), shutdown).await;
        drop(session_guard);
        result
    } else {
        pipe_pool(socket, stream, initial_data, Some(recorder)).await
    }
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
        .mux();
    let request = OpenRequest {
        tunnel_id: route.tunnel_id.clone(),
        protocol,
        target,
        remote_addr,
        timeout_ms: 5000,
    };
    let payload = serde_json::to_vec(&request)?;
    let mut stream = mux.open_stream(Bytes::from(payload)).await?;
    let response = tokio::time::timeout(
        Duration::from_millis(request.timeout_ms.max(1)),
        stream.recv_data(),
    )
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "agent open target {} timed out after {}ms",
                request.target, request.timeout_ms
            ),
        )
    })?
    .ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "agent closed stream before open response",
        )
    })?;
    let response: OpenResponse = serde_json::from_slice(&response).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid agent open response: {err}"),
        )
    })?;
    if !response.ok {
        anyhow::bail!(
            "agent failed to open target {}: {}",
            request.target,
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
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

/// Acquire a pre-warmed, mux-free TCP channel from the agent connection pool and
/// negotiate the target on it. The returned stream is a dedicated noise duplex
/// channel with no head-of-line blocking across concurrent requests.
pub async fn open_pool_stream(
    state: AppState,
    route: &StreamRoute,
    target: String,
    remote_addr: String,
) -> anyhow::Result<DuplexStream> {
    let pool_rx = state
        .clients
        .get(&route.client_id)
        .ok_or_else(|| anyhow::anyhow!("client {} is offline", route.client_id))?
        .pool_rx();

    let request = OpenRequest {
        tunnel_id: route.tunnel_id.clone(),
        protocol: TargetProtocol::Tcp,
        target,
        remote_addr,
        timeout_ms: 5000,
    };

    for _ in 0..POOL_ACQUIRE_RETRIES {
        let mut stream = {
            let mut guard = pool_rx.lock().await;
            match tokio::time::timeout(POOL_ACQUIRE_TIMEOUT, guard.recv()).await {
                Ok(Some(stream)) => stream,
                Ok(None) => anyhow::bail!("client {} pool is closed", route.client_id),
                Err(_) => anyhow::bail!(
                    "client {} pool exhausted: no idle connection within {:?}",
                    route.client_id,
                    POOL_ACQUIRE_TIMEOUT
                ),
            }
        };

        // A pooled connection may have died while idle. Detect that on write/read
        // and transparently fall through to the next pooled connection.
        if let Err(err) = write_json(&mut stream, &request).await {
            debug!(error = %err, "pooled connection dead on write, retrying");
            continue;
        }
        let response: OpenResponse = match tokio::time::timeout(
            Duration::from_millis(request.timeout_ms.max(1)),
            read_json(&mut stream),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                debug!(error = %err, "pooled connection dead on read, retrying");
                continue;
            }
            Err(_) => anyhow::bail!(
                "agent open target {} timed out after {}ms",
                request.target,
                request.timeout_ms
            ),
        };
        if !response.ok {
            anyhow::bail!(
                "agent failed to open target {}: {}",
                request.target,
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            );
        }

        let db = state.db.clone();
        let client_id = route.client_id.clone();
        let tunnel_id = route.tunnel_id.clone();
        let protocol = request.protocol.clone();
        let target = request.target.clone();
        let remote_addr = request.remote_addr.clone();
        tokio::spawn(async move {
            if let Err(err) = db
                .record_stream_open(&client_id, &tunnel_id, &protocol, &target, &remote_addr)
                .await
            {
                debug!(error = %err, "failed to record stream session");
            }
        });

        return Ok(stream);
    }

    anyhow::bail!(
        "no live pooled connection available for client {}",
        route.client_id
    )
}

pub async fn pipe_pool(
    socket: TcpStream,
    stream: DuplexStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
) -> anyhow::Result<()> {
    pipe_pool_inner(socket, stream, initial_data, recorder, None).await
}

pub async fn pipe_pool_with_shutdown(
    socket: TcpStream,
    stream: DuplexStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    pipe_pool_inner(socket, stream, initial_data, recorder, Some(shutdown)).await
}

async fn pipe_pool_inner(
    socket: TcpStream,
    stream: DuplexStream,
    initial_data: Option<Bytes>,
    recorder: Option<TrafficRecorder>,
    shutdown: Option<watch::Receiver<bool>>,
) -> anyhow::Result<()> {
    socket.set_nodelay(true)?;
    let (mut tcp_read, mut tcp_write) = socket.into_split();
    let (mut stream_read, mut stream_write) = tokio::io::split(stream);

    if let Some(data) = initial_data {
        if let Some(recorder) = &recorder {
            recorder.add(0, data.len() as u64);
        }
        stream_write.write_all(&data).await?;
    }

    // Uplink: client -> target (tx direction).
    let mut uplink = {
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let mut buf = vec![0_u8; POOL_COPY_BUF_SIZE];
            loop {
                let n = tcp_read.read(&mut buf).await?;
                if n == 0 {
                    let _ = stream_write.shutdown().await; // half-close: send FIN to agent
                    break;
                }
                stream_write.write_all(&buf[..n]).await?;
                if let Some(recorder) = &recorder {
                    recorder.add(0, n as u64);
                }
            }
            anyhow::Ok(())
        })
    };

    // Downlink: target -> client (rx direction).
    let mut downlink = {
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let mut buf = vec![0_u8; POOL_COPY_BUF_SIZE];
            loop {
                let n = stream_read.read(&mut buf).await?;
                if n == 0 {
                    let _ = tcp_write.shutdown().await; // half-close: send FIN to client
                    break;
                }
                tcp_write.write_all(&buf[..n]).await?;
                if let Some(recorder) = &recorder {
                    recorder.add(n as u64, 0);
                }
            }
            anyhow::Ok(())
        })
    };

    if let Some(mut shutdown) = shutdown {
        tokio::select! {
            result = &mut uplink => {
                if matches!(result, Err(_) | Ok(Err(_))) {
                    downlink.abort();
                }
                let res2 = downlink.await;
                finish_pipe_task(result)?;
                finish_pipe_task(res2)?;
            }
            result = &mut downlink => {
                if matches!(result, Err(_) | Ok(Err(_))) {
                    uplink.abort();
                }
                let res2 = uplink.await;
                finish_pipe_task(result)?;
                finish_pipe_task(res2)?;
            }
            _ = shutdown.changed() => {
                uplink.abort();
                downlink.abort();
                let _ = uplink.await;
                let _ = downlink.await;
            }
        }
    } else {
        tokio::select! {
            result = &mut uplink => {
                if matches!(result, Err(_) | Ok(Err(_))) {
                    downlink.abort();
                }
                let res2 = downlink.await;
                finish_pipe_task(result)?;
                finish_pipe_task(res2)?;
            }
            result = &mut downlink => {
                if matches!(result, Err(_) | Ok(Err(_))) {
                    uplink.abort();
                }
                let res2 = uplink.await;
                finish_pipe_task(result)?;
                finish_pipe_task(res2)?;
            }
        }
    }
    Ok(())
}

fn finish_pipe_task(
    result: Result<anyhow::Result<()>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    result??;
    Ok(())
}
