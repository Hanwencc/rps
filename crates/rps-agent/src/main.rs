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
use tracing::{info, warn};

const TCP_COPY_BUF_SIZE: usize = 64 * 1024;

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

    tokio::spawn(run_control(control));

    let mut mux = Mux::new(data);
    while let Some(stream) = mux.accept().await {
        tokio::spawn(async move {
            if let Err(err) = handle_stream(stream).await {
                warn!(error = %err, "mux stream failed");
            }
        });
    }
    Ok(())
}

async fn connect_role(config: &AgentConfig, role: HelloRole) -> anyhow::Result<DuplexStream> {
    let mut stream = TcpStream::connect(&config.server_addr).await?;
    stream.set_nodelay(true)?;
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
    target.set_nodelay(true)?;
    send_open_ok(&writer).await?;
    let (mut target_read, mut target_write) = target.into_split();

    let writer_up = writer.clone();
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
