use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use rps_core::{
    config::{AgentConfig, AgentConfigRoot, load_agent_config},
    protocol::{
        ControlMessage, Hello, HelloAck, HelloRole, OpenRequest, TargetProtocol, read_json,
        write_json,
    },
};
use rps_mux::Mux;
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};
use tracing::{info, warn};

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
    let env_vkey = env_value("vkey").or_else(|| env_value("RPS_VKEY"));
    let env_reconnect_interval =
        env_value("reconnect_interval_secs").or_else(|| env_value("RPS_RECONNECT_INTERVAL_SECS"));

    if env_server_addr.is_some() || env_vkey.is_some() || env_reconnect_interval.is_some() {
        let server_addr =
            env_server_addr.ok_or_else(|| anyhow::anyhow!("agent env server_addr is required"))?;
        let vkey = env_vkey.ok_or_else(|| anyhow::anyhow!("agent env vkey is required"))?;
        let reconnect_interval_secs = env_reconnect_interval
            .map(|value| value.parse().context("invalid reconnect_interval_secs env"))
            .transpose()?
            .unwrap_or(5);
        return Ok(AgentConfigRoot {
            agent: AgentConfig {
                server_addr,
                vkey,
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
    info!(server_addr = %config.server_addr, "connecting rps-controller");
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

async fn connect_role(config: &AgentConfig, role: HelloRole) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(&config.server_addr).await?;
    let hello = Hello::new(role, config.vkey.clone());
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

async fn run_control(mut stream: TcpStream) {
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
    let target = TcpStream::connect(&request.target).await?;
    let (mut target_read, mut target_write) = target.into_split();

    let uplink = tokio::spawn(async move {
        while let Some(data) = reader.recv_data().await {
            target_write.write_all(&data).await?;
        }
        anyhow::Ok(())
    });

    let downlink = tokio::spawn(async move {
        let mut buf = vec![0_u8; 16 * 1024];
        loop {
            let n = target_read.read(&mut buf).await?;
            if n == 0 {
                writer.close().await?;
                break;
            }
            writer.send_data(Bytes::copy_from_slice(&buf[..n])).await?;
        }
        anyhow::Ok(())
    });

    let _ = tokio::try_join!(uplink, downlink)?;
    Ok(())
}

async fn handle_udp_target(
    writer: rps_mux::MuxStreamWriter,
    mut reader: rps_mux::MuxStreamReader,
    request: OpenRequest,
) -> anyhow::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(&request.target).await?;
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
