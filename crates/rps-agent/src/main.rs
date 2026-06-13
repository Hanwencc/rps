use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use rps_core::{
    config::{AgentConfig, AgentConfigRoot, load_agent_config},
    noise,
    protocol::{
        ControlMessage, Hello, HelloAck, HelloRole, NoisePrelude, OpenRequest, OpenResponse,
        TargetProtocol, read_json, write_json,
    },
};
use rps_mux::Mux;
use std::{
    env,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    net::{TcpStream, UdpSocket},
};
use tracing::{debug, info, warn};

const TCP_COPY_BUF_SIZE: usize = 64 * 1024;
const POOL_COPY_BUF_SIZE: usize = 128 * 1024;
/// Number of pre-warmed mux-free TCP connections kept ready for bulk traffic.
const POOL_SIZE: usize = 32;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "configs/agent.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let root = load_agent_config_with_env(&args.config)?;
    loop {
        if let Err(err) = run_agent(root.agent.clone()).await {
            warn!(error = %err, "agent session ended");
        }
        tokio::time::sleep(std::time::Duration::from_secs(
            root.agent.reconnect_interval_secs,
        ))
        .await;
    }
}

fn load_agent_config_with_env(path: &str) -> anyhow::Result<AgentConfigRoot> {
    let env_server_addr = env_value("server_addr").or_else(|| env_value("RPS_SERVER_ADDR"));
    let env_client_id = env_value("client_id").or_else(|| env_value("RPS_CLIENT_ID"));
    let env_psk = env_value("psk").or_else(|| env_value("RPS_PSK"));
    let env_reconnect_interval =
        env_value("reconnect_interval_secs").or_else(|| env_value("RPS_RECONNECT_INTERVAL_SECS"));

    if env_server_addr.is_some()
        || env_client_id.is_some()
        || env_psk.is_some()
        || env_reconnect_interval.is_some()
    {
        let server_addr =
            env_server_addr.ok_or_else(|| anyhow::anyhow!("agent env server_addr is required"))?;
        let client_id =
            env_client_id.ok_or_else(|| anyhow::anyhow!("agent env client_id is required"))?;
        let psk = env_psk.ok_or_else(|| anyhow::anyhow!("agent env psk is required"))?;
        let reconnect_interval_secs = env_reconnect_interval
            .map(|value| value.parse().context("invalid reconnect_interval_secs env"))
            .transpose()?
            .unwrap_or(5);
        return Ok(AgentConfigRoot {
            agent: AgentConfig {
                server_addr,
                client_id,
                psk,
                reconnect_interval_secs,
            },
        });
    }

    load_agent_config(path)
}

fn env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn run_agent(config: AgentConfig) -> anyhow::Result<()> {
    info!(server_addr = %config.server_addr, client_id = %config.client_id, "connecting rps-controller");
    let control = connect_role(&config, HelloRole::Control).await?;
    let data = connect_role(&config, HelloRole::Data).await?;

    // Background tasks (control heartbeat + elastic pool manager) are tracked so
    // they are torn down together when the data mux session ends.
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        run_control(control).await;
    });
    {
        let config = config.clone();
        tasks.spawn(async move {
            run_pool_manager(config).await;
        });
    }

    // The data mux still carries UDP datagrams and any control-plane streams.
    let mut mux = Mux::new(data);
    while let Some(stream) = mux.accept().await {
        tokio::spawn(async move {
            if let Err(err) = handle_stream(stream).await {
                warn!(error = %err, "mux stream failed");
            }
        });
    }

    tasks.abort_all();
    Ok(())
}

/// Elastic pre-warmed pool manager.
///
/// Keeps exactly `POOL_SIZE` warm (idle) connections parked in the controller's
/// pool at all times, while the number of *active* relays it serves is unbounded.
/// A warm connection holds a semaphore permit; the moment the controller assigns
/// it a target the permit is released so a replacement warm connection is dialled
/// immediately, and the now-busy connection is detached as an independent relay.
/// This decouples "warm buffer size" from "concurrent active connections", which
/// is what previously caused pool exhaustion under many long-lived connections.
async fn run_pool_manager(config: AgentConfig) {
    let warm_slots = std::sync::Arc::new(tokio::sync::Semaphore::new(POOL_SIZE));
    loop {
        let permit = match warm_slots.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let config = config.clone();
        tokio::spawn(async move {
            // Establish a warm pooled connection. The permit is held while it is idle.
            let mut stream = match connect_role(&config, HelloRole::Pool).await {
                Ok(stream) => stream,
                Err(err) => {
                    warn!(error = %err, "pool warm connect failed");
                    // Back off while still holding the permit so we don't hammer the
                    // controller when it is unreachable.
                    tokio::time::sleep(Duration::from_secs(
                        config.reconnect_interval_secs.max(1),
                    ))
                    .await;
                    drop(permit);
                    return;
                }
            };

            // Block until the controller assigns a target (connection is consumed).
            let request: OpenRequest = match read_json(&mut stream).await {
                Ok(request) => request,
                Err(err) => {
                    // Idle warm connection died, commonly a cross-border idle reset.
                    // Recycle it quietly; the manager will dial a replacement.
                    debug!(error = %err, "warm pool connection recycled before use");
                    drop(permit);
                    return;
                }
            };

            // Consumed: free the warm slot so a replacement is dialled right away,
            // then serve this request as an unbounded, dedicated relay.
            drop(permit);
            if let Err(err) = serve_pool_request(stream, request).await {
                debug!(error = %err, "pool relay ended");
            }
        });
    }
}

async fn serve_pool_request(stream: DuplexStream, request: OpenRequest) -> anyhow::Result<()> {
    match request.protocol {
        TargetProtocol::Tcp => handle_pool_tcp(stream, request).await,
        TargetProtocol::Udp => {
            let mut stream = stream;
            let _ = write_json(
                &mut stream,
                &OpenResponse::err("udp is not supported on pool channel"),
            )
            .await;
            anyhow::bail!("udp request received on pool channel");
        }
    }
}

async fn handle_pool_tcp(mut stream: DuplexStream, request: OpenRequest) -> anyhow::Result<()> {
    let target =
        match tokio::time::timeout(open_timeout(&request), TcpStream::connect(&request.target))
            .await
        {
            Ok(Ok(target)) => target,
            Ok(Err(err)) => {
                let _ = write_json(
                    &mut stream,
                    &OpenResponse::err(format!("tcp connect {} failed: {err}", request.target)),
                )
                .await;
                return Err(err.into());
            }
            Err(_) => {
                let error = format!(
                    "tcp connect {} timed out after {}ms",
                    request.target, request.timeout_ms
                );
                let _ = write_json(&mut stream, &OpenResponse::err(error.clone())).await;
                anyhow::bail!(error);
            }
        };
    rps_core::net::tune_cross_border(&target)?;
    write_json(&mut stream, &OpenResponse::ok()).await?;

    let (mut stream_read, mut stream_write) = tokio::io::split(stream);
    let (mut target_read, mut target_write) = target.into_split();

    // Uplink: controller -> target.
    let mut uplink = tokio::spawn(async move {
        let mut buf = vec![0_u8; POOL_COPY_BUF_SIZE];
        loop {
            let n = stream_read.read(&mut buf).await?;
            if n == 0 {
                let _ = target_write.shutdown().await;
                break;
            }
            target_write.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    });

    // Downlink: target -> controller.
    let mut downlink = tokio::spawn(async move {
        let mut buf = vec![0_u8; POOL_COPY_BUF_SIZE];
        loop {
            let n = target_read.read(&mut buf).await?;
            if n == 0 {
                let _ = stream_write.shutdown().await;
                break;
            }
            stream_write.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    });

    tokio::select! {
        result = &mut uplink => {
            if matches!(result, Err(_) | Ok(Err(_))) {
                downlink.abort();
            }
            let res2 = downlink.await;
            result??;
            if let Ok(Err(e)) = res2 {
                return Err(e);
            }
        }
        result = &mut downlink => {
            if matches!(result, Err(_) | Ok(Err(_))) {
                uplink.abort();
            }
            let res2 = uplink.await;
            result??;
            if let Ok(Err(e)) = res2 {
                return Err(e);
            }
        }
    }
    Ok(())
}

async fn connect_role(config: &AgentConfig, role: HelloRole) -> anyhow::Result<DuplexStream> {
    let stream = TcpStream::connect(&config.server_addr).await?;
    rps_core::net::tune_cross_border(&stream)?;
    let mut stream = stream;
    let prelude = NoisePrelude::new(config.client_id.clone());
    write_json(&mut stream, &prelude).await?;

    let mut stream = noise::connect(stream, &config.psk).await?;
    let hello = Hello::new(role, config.client_id.clone());
    write_json(&mut stream, &hello).await?;
    let ack: HelloAck = read_json(&mut stream).await?;
    if !ack.ok {
        anyhow::bail!(
            "controller rejected handshake: {}",
            ack.error.unwrap_or_else(|| "unknown error".to_string())
        );
    }
    Ok(stream)
}

async fn run_control(mut stream: DuplexStream) {
    loop {
        let ts = now_secs();
        if let Err(err) = write_json(&mut stream, &ControlMessage::Ping { ts }).await {
            warn!(error = %err, "control ping failed");
            break;
        }
        match read_json::<_, ControlMessage>(&mut stream).await {
            Ok(ControlMessage::Pong { .. }) => {}
            Ok(_) => {}
            Err(err) => {
                warn!(error = %err, "control read failed");
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

async fn handle_stream(stream: rps_mux::MuxStream) -> anyhow::Result<()> {
    let (writer, mut reader) = stream.split();
    let open = reader
        .recv_data()
        .await
        .ok_or_else(|| anyhow::anyhow!("missing open payload"))?;
    let request: OpenRequest = serde_json::from_slice(&open).context("invalid open payload")?;
    match request.protocol {
        TargetProtocol::Tcp => handle_tcp_target(writer, reader, request).await,
        TargetProtocol::Udp => handle_udp_target(writer, reader, request).await,
    }
}

async fn handle_tcp_target(
    writer: rps_mux::MuxStreamWriter,
    mut reader: rps_mux::MuxStreamReader,
    request: OpenRequest,
) -> anyhow::Result<()> {
    let target =
        match tokio::time::timeout(open_timeout(&request), TcpStream::connect(&request.target))
            .await
        {
            Ok(Ok(target)) => target,
            Ok(Err(err)) => {
                send_open_error(
                    &writer,
                    format!("tcp connect {} failed: {err}", request.target),
                )
                .await?;
                return Err(err.into());
            }
            Err(_) => {
                let error = format!(
                    "tcp connect {} timed out after {}ms",
                    request.target, request.timeout_ms
                );
                send_open_error(&writer, error.clone()).await?;
                anyhow::bail!(error);
            }
        };
    rps_core::net::tune_cross_border(&target)?;
    send_open_ok(&writer).await?;
    let (mut target_read, mut target_write) = target.into_split();

    let mut uplink = tokio::spawn(async move {
        while let Some(data) = reader.recv_data().await {
            if let Err(e) = target_write.write_all(&data).await {
                return Err(e.into());
            }
        }
        let _ = target_write.shutdown().await;
        anyhow::Ok(())
    });

    let mut downlink = tokio::spawn(async move {
        let mut buf = vec![0_u8; TCP_COPY_BUF_SIZE];
        loop {
            let n = target_read.read(&mut buf).await?;
            if n == 0 {
                let _ = writer.close().await;
                break;
            }
            if let Err(e) = writer.send_data(Bytes::copy_from_slice(&buf[..n])).await {
                return Err(e.into());
            }
        }
        anyhow::Ok(())
    });

    tokio::select! {
        result = &mut uplink => {
            if matches!(result, Err(_) | Ok(Err(_))) {
                downlink.abort();
            }
            let res2 = downlink.await;
            result??;
            if let Ok(Err(e)) = res2 {
                return Err(e);
            }
        }
        result = &mut downlink => {
            if matches!(result, Err(_) | Ok(Err(_))) {
                uplink.abort();
            }
            let res2 = uplink.await;
            result??;
            if let Ok(Err(e)) = res2 {
                return Err(e);
            }
        }
    }
    Ok(())
}

async fn handle_udp_target(
    writer: rps_mux::MuxStreamWriter,
    mut reader: rps_mux::MuxStreamReader,
    request: OpenRequest,
) -> anyhow::Result<()> {
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(socket) => socket,
        Err(err) => {
            send_open_error(&writer, format!("udp bind failed: {err}")).await?;
            return Err(err.into());
        }
    };
    match tokio::time::timeout(open_timeout(&request), socket.connect(&request.target)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            send_open_error(
                &writer,
                format!("udp connect {} failed: {err}", request.target),
            )
            .await?;
            return Err(err.into());
        }
        Err(_) => {
            let error = format!(
                "udp connect {} timed out after {}ms",
                request.target, request.timeout_ms
            );
            send_open_error(&writer, error.clone()).await?;
            anyhow::bail!(error);
        }
    }
    send_open_ok(&writer).await?;
    let socket = std::sync::Arc::new(socket);

    let uplink_socket = socket.clone();
    let uplink = tokio::spawn(async move {
        while let Some(data) = reader.recv_data().await {
            uplink_socket.send(&data).await?;
        }
        anyhow::Ok(())
    });

    let downlink_socket = socket.clone();
    let downlink = tokio::spawn(async move {
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            let n = downlink_socket.recv(&mut buf).await?;
            writer.send_data(Bytes::copy_from_slice(&buf[..n])).await?;
        }
        #[allow(unreachable_code)]
        anyhow::Ok(())
    });

    let _ = tokio::try_join!(uplink, downlink)?;
    Ok(())
}

async fn send_open_ok(writer: &rps_mux::MuxStreamWriter) -> anyhow::Result<()> {
    writer
        .send_data(Bytes::from(serde_json::to_vec(&OpenResponse::ok())?))
        .await?;
    Ok(())
}

async fn send_open_error(
    writer: &rps_mux::MuxStreamWriter,
    error: impl Into<String>,
) -> anyhow::Result<()> {
    writer
        .send_data(Bytes::from(serde_json::to_vec(&OpenResponse::err(
            error.into(),
        ))?))
        .await?;
    let _ = writer.close().await;
    Ok(())
}

fn open_timeout(request: &OpenRequest) -> Duration {
    Duration::from_millis(request.timeout_ms.max(1))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
