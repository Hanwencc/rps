use crate::{AppState, db::DbTunnel, proxy_tcp, proxy_udp};
use anyhow::Context;
use dashmap::DashMap;
use rps_core::config::TunnelConfig;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::{Mutex, watch},
    task::JoinHandle,
};
use tracing::{info, warn};

pub(crate) struct TunnelManager {
    running: Mutex<HashMap<String, TunnelRuntime>>,
}

struct TunnelRuntime {
    shutdown: watch::Sender<bool>,
    handle: JoinHandle<()>,
    listen: String,
    sessions: Arc<DashMap<String, watch::Sender<bool>>>,
    active_connections: Arc<AtomicUsize>,
}

pub(crate) struct TunnelSessionGuard {
    id: String,
    sessions: Arc<DashMap<String, watch::Sender<bool>>>,
    _connection_guard: TunnelConnectionGuard,
    shutdown: watch::Receiver<bool>,
}

pub(crate) struct TunnelConnectionGuard {
    active_connections: Arc<AtomicUsize>,
}

impl TunnelManager {
    pub(crate) fn new() -> Self {
        Self {
            running: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn start_enabled_from_db(&self, state: AppState) -> anyhow::Result<()> {
        let tunnels = state.db.list_tunnels().await?;
        for tunnel in tunnels.into_iter().filter(|tunnel| tunnel.enabled) {
            let config = tunnel_config_from_db(&tunnel);
            if let Err(err) = self.start(state.clone(), config).await {
                warn!(tunnel_id = %tunnel.id, error = %err, "failed to start configured tunnel");
            }
        }
        Ok(())
    }

    pub(crate) async fn start(&self, state: AppState, tunnel: TunnelConfig) -> anyhow::Result<()> {
        let mut running = self.running.lock().await;
        if running.contains_key(&tunnel.id) {
            anyhow::bail!("tunnel {} is already running", tunnel.id);
        }
        if !state
            .policy
            .allowed(&crate::policy::tunnel_key(tunnel.id.clone()))
        {
            anyhow::bail!("tunnel {} is disabled by policy", tunnel.id);
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let id = tunnel.id.clone();
        let listen = tunnel.listen.clone();
        let sessions = Arc::new(DashMap::new());
        let active_connections = Arc::new(AtomicUsize::new(0));
        let handle = match tunnel.mode {
            rps_core::config::TunnelMode::Tcp => {
                let listener = TcpListener::bind(&tunnel.listen)
                    .await
                    .with_context(|| format!("failed to bind tcp tunnel {}", tunnel.listen))?;
                tokio::spawn(async move {
                    if let Err(err) = proxy_tcp::serve(state, tunnel, listener, shutdown_rx).await {
                        warn!(error = %err, "tcp tunnel task stopped");
                    }
                })
            }
            rps_core::config::TunnelMode::Udp => {
                let socket = UdpSocket::bind(&tunnel.listen)
                    .await
                    .with_context(|| format!("failed to bind udp tunnel {}", tunnel.listen))?;
                tokio::spawn(async move {
                    if let Err(err) = proxy_udp::serve(state, tunnel, socket, shutdown_rx).await {
                        warn!(error = %err, "udp tunnel task stopped");
                    }
                })
            }
        };

        running.insert(
            id.clone(),
            TunnelRuntime {
                shutdown: shutdown_tx,
                handle,
                listen,
                sessions,
                active_connections,
            },
        );
        info!(tunnel_id = %id, "tunnel runtime started");
        Ok(())
    }

    pub(crate) async fn stop(&self, id: &str) -> anyhow::Result<bool> {
        let Some(runtime) = self.running.lock().await.remove(id) else {
            return Ok(false);
        };

        let _ = runtime.shutdown.send(true);
        for entry in runtime.sessions.iter() {
            let _ = entry.value().send(true);
        }
        let mut handle = runtime.handle;
        if tokio::time::timeout(Duration::from_secs(5), &mut handle)
            .await
            .is_err()
        {
            warn!(tunnel_id = %id, listen = %runtime.listen, "tunnel did not stop in time, aborting task");
            handle.abort();
            let _ = handle.await;
        }
        info!(tunnel_id = %id, listen = %runtime.listen, "tunnel runtime stopped");
        Ok(true)
    }

    pub(crate) async fn register_session(&self, id: &str) -> Option<TunnelSessionGuard> {
        let running = self.running.lock().await;
        let runtime = running.get(id)?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let session_id = uuid::Uuid::new_v4().to_string();
        runtime.sessions.insert(session_id.clone(), shutdown_tx);
        Some(TunnelSessionGuard {
            id: session_id,
            sessions: runtime.sessions.clone(),
            _connection_guard: TunnelConnectionGuard::new(runtime.active_connections.clone()),
            shutdown: shutdown_rx,
        })
    }

    pub(crate) async fn register_udp_session(&self, id: &str) -> Option<TunnelConnectionGuard> {
        let running = self.running.lock().await;
        let runtime = running.get(id)?;
        Some(TunnelConnectionGuard::new(
            runtime.active_connections.clone(),
        ))
    }

    pub(crate) async fn active_count(&self, id: &str) -> usize {
        let running = self.running.lock().await;
        running
            .get(id)
            .map(|runtime| runtime.active_connections.load(Ordering::Relaxed))
            .unwrap_or_default()
    }
}

impl TunnelSessionGuard {
    pub(crate) fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown.clone()
    }
}

impl Drop for TunnelSessionGuard {
    fn drop(&mut self) {
        self.sessions.remove(&self.id);
    }
}

impl TunnelConnectionGuard {
    fn new(active_connections: Arc<AtomicUsize>) -> Self {
        active_connections.fetch_add(1, Ordering::Relaxed);
        Self { active_connections }
    }
}

impl Drop for TunnelConnectionGuard {
    fn drop(&mut self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) fn tunnel_config_from_db(tunnel: &DbTunnel) -> TunnelConfig {
    TunnelConfig {
        id: tunnel.id.clone(),
        client_id: tunnel.client_id.clone(),
        mode: tunnel.mode.clone(),
        listen: tunnel.listen.clone(),
        target: tunnel.target.clone(),
        enabled: tunnel.enabled,
    }
}
