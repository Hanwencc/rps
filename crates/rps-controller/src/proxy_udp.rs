use crate::{AppState, proxy_tcp};
use bytes::Bytes;
use dashmap::DashMap;
use rps_core::{
    config::TunnelConfig,
    protocol::{OpenRequest, TargetProtocol},
};
use rps_mux::MuxStreamWriter;
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::UdpSocket;
use tracing::{error, info, warn};

const UDP_IDLE_SECS: u64 = 120;

struct UdpSession {
    writer: MuxStreamWriter,
    last_seen: Arc<AtomicU64>,
}

pub async fn run(state: AppState, tunnel: TunnelConfig) {
    if let Err(err) = run_inner(state, tunnel).await {
        error!(error = %err, "udp proxy stopped");
    }
}

async fn run_inner(state: AppState, tunnel: TunnelConfig) -> anyhow::Result<()> {
    let socket = Arc::new(UdpSocket::bind(&tunnel.listen).await?);
    let sessions = Arc::new(DashMap::<SocketAddr, UdpSession>::new());
    let route = proxy_tcp::StreamRoute {
        tunnel_id: tunnel.id.clone(),
        client_id: tunnel.client_id.clone(),
    };
    let recorder = proxy_tcp::TrafficRecorder::new(&state, &route);
    info!(tunnel_id = %tunnel.id, listen = %tunnel.listen, "udp proxy listening");

    {
        let sessions = sessions.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let now = now_secs();
                let stale: Vec<_> = sessions
                    .iter()
                    .filter_map(|entry| {
                        let last_seen = entry.value().last_seen.load(Ordering::Relaxed);
                        (now.saturating_sub(last_seen) > UDP_IDLE_SECS).then_some(*entry.key())
                    })
                    .collect();
                for key in stale {
                    if let Some((_, session)) = sessions.remove(&key) {
                        let _ = session.writer.close().await;
                    }
                }
            }
        });
    }

    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let (n, remote_addr) = socket.recv_from(&mut buf).await?;
        let data = Bytes::copy_from_slice(&buf[..n]);
        let writer = if let Some(session) = sessions.get(&remote_addr) {
            session.last_seen.store(now_secs(), Ordering::Relaxed);
            session.writer.clone()
        } else {
            let target = tunnel
                .target
                .clone()
                .ok_or_else(|| anyhow::anyhow!("udp tunnel target is required"))?;
            let mux = state
                .clients
                .get(&tunnel.client_id)
                .ok_or_else(|| anyhow::anyhow!("client {} is offline", tunnel.client_id))?
                .clone();
            let request = OpenRequest {
                tunnel_id: tunnel.id.clone(),
                protocol: TargetProtocol::Udp,
                target,
                remote_addr: remote_addr.to_string(),
                timeout_ms: 5000,
            };
            let stream = mux
                .open_stream(Bytes::from(serde_json::to_vec(&request)?))
                .await?;
            if let Err(err) = state
                .db
                .record_stream_open(
                    &tunnel.client_id,
                    &tunnel.id,
                    &request.protocol,
                    &request.target,
                    &request.remote_addr,
                )
                .await
            {
                warn!(%remote_addr, error = %err, "failed to record udp stream session");
            }
            let (writer, mut reader) = stream.split();
            let last_seen = Arc::new(AtomicU64::new(now_secs()));
            sessions.insert(
                remote_addr,
                UdpSession {
                    writer: writer.clone(),
                    last_seen: last_seen.clone(),
                },
            );
            let socket = socket.clone();
            let sessions = sessions.clone();
            let response_recorder = recorder.clone();
            tokio::spawn(async move {
                while let Some(packet) = reader.recv_data().await {
                    if let Err(err) = socket.send_to(&packet, remote_addr).await {
                        warn!(%remote_addr, error = %err, "udp response write failed");
                        break;
                    }
                    response_recorder.add(packet.len() as u64, 0).await;
                    last_seen.store(now_secs(), Ordering::Relaxed);
                }
                sessions.remove(&remote_addr);
            });
            writer
        };
        let data_len = data.len() as u64;
        if let Err(err) = writer.send_data(data).await {
            warn!(%remote_addr, error = %err, "udp send to mux failed");
            sessions.remove(&remote_addr);
        } else {
            recorder.add(0, data_len).await;
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
