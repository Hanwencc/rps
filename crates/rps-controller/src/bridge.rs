use crate::{AppState, ClientConnection};
use rps_core::noise;
use rps_core::protocol::{
    ControlMessage, Hello, HelloAck, HelloRole, MAGIC, NoisePrelude, read_json, write_json,
};
use rps_mux::Mux;
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

pub async fn run(state: AppState) {
    if let Err(err) = run_inner(state).await {
        error!(error = %err, "bridge listener stopped");
    }
}

async fn run_inner(state: AppState) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&state.config.server.bridge_addr).await?;
    loop {
        let (stream, remote_addr) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(state, stream, remote_addr.to_string()).await {
                warn!(%remote_addr, error = %err, "bridge connection failed");
            }
        });
    }
}

async fn handle_conn(
    state: AppState,
    mut stream: TcpStream,
    remote_addr: String,
) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    let prelude: NoisePrelude = read_json(&mut stream).await?;
    if prelude.magic != MAGIC {
        anyhow::bail!("bad magic");
    }
    let client = state
        .db
        .find_enabled_client_by_id(&prelude.client_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("invalid client id"))?;

    let mut stream = noise::accept(stream, &client.psk).await?;
    let hello: Hello = read_json(&mut stream).await?;
    if hello.magic != MAGIC {
        write_json(&mut stream, &HelloAck::err("bad magic")).await?;
        return Ok(());
    }
    if hello.client_id != client.id {
        write_json(&mut stream, &HelloAck::err("client id mismatch")).await?;
        return Ok(());
    }

    write_json(&mut stream, &HelloAck::ok()).await?;

    match hello.role {
        HelloRole::Control => {
            info!(client_id = %client.id, "agent control connected");
            let session_id = state
                .db
                .record_agent_connected(&client.id, "control", &remote_addr)
                .await?;
            loop {
                match read_json::<_, ControlMessage>(&mut stream).await {
                    Ok(ControlMessage::Ping { ts }) => {
                        write_json(&mut stream, &ControlMessage::Pong { ts }).await?;
                    }
                    Ok(ControlMessage::Pong { .. }) => {}
                    Ok(ControlMessage::Shutdown { reason }) => {
                        info!(client_id = %client.id, %reason, "agent requested shutdown");
                        break;
                    }
                    Err(err) => {
                        warn!(client_id = %client.id, error = %err, "agent control disconnected");
                        break;
                    }
                }
            }
            let _ = state
                .db
                .record_agent_disconnected(&session_id, &client.id)
                .await;
        }
        HelloRole::Data => {
            info!(client_id = %client.id, "agent data mux connected");
            let session_id = state
                .db
                .record_agent_connected(&client.id, "data", &remote_addr)
                .await?;
            let mut mux = Mux::new(stream);
            state.clients.insert(
                client.id.clone(),
                ClientConnection::new(mux.handle(), session_id.clone()),
            );
            while let Some(stream) = mux.accept().await {
                warn!(
                    client_id = %client.id,
                    stream_id = stream.id(),
                    "unexpected inbound stream from agent"
                );
            }
            warn!(client_id = %client.id, "agent data mux disconnected");
            state.clients.remove_if(&client.id, |_, current| {
                current.data_session_id() == session_id
            });
            let _ = state
                .db
                .record_agent_disconnected(&session_id, &client.id)
                .await;
        }
    }

    Ok(())
}
