use crate::{AppState, proxy_http, proxy_socks5};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::info;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct ProxyManager {
    sessions: Arc<DashMap<String, ProxySessionEntry>>,
}

struct ProxySessionEntry {
    account_id: String,
    shutdown: watch::Sender<bool>,
}

pub(crate) struct ProxySessionGuard {
    id: Option<String>,
    shutdown: watch::Receiver<bool>,
    _anonymous_shutdown: Option<watch::Sender<bool>>,
    sessions: Arc<DashMap<String, ProxySessionEntry>>,
}

impl ProxyManager {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn start_from_config(&self, state: AppState) {
        if let Some(proxy) = state.config.server.http_proxy.clone().filter(|p| p.enabled) {
            tokio::spawn(proxy_http::run(state.clone(), proxy));
        }

        if let Some(proxy) = state.config.server.socks5.clone().filter(|p| p.enabled) {
            tokio::spawn(proxy_socks5::run(state, proxy));
        }
    }

    pub(crate) fn register(&self, account_id: Option<String>) -> ProxySessionGuard {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut anonymous_shutdown = Some(shutdown_tx);
        let id = account_id.as_ref().map(|account_id| {
            let id = Uuid::new_v4().to_string();
            let shutdown = anonymous_shutdown
                .take()
                .expect("shutdown sender is available before session insert");
            self.sessions.insert(
                id.clone(),
                ProxySessionEntry {
                    account_id: account_id.clone(),
                    shutdown,
                },
            );
            id
        });
        ProxySessionGuard {
            id,
            shutdown: shutdown_rx,
            _anonymous_shutdown: anonymous_shutdown,
            sessions: self.sessions.clone(),
        }
    }

    pub(crate) fn revoke_account(&self, account_id: &str) -> usize {
        let session_ids: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (entry.value().account_id == account_id).then_some(entry.key().clone())
            })
            .collect();

        for id in &session_ids {
            if let Some((_, session)) = self.sessions.remove(id) {
                let _ = session.shutdown.send(true);
            }
        }

        if !session_ids.is_empty() {
            info!(
                account_id,
                count = session_ids.len(),
                "proxy account sessions revoked"
            );
        }
        session_ids.len()
    }

    pub(crate) fn active_count(&self, account_id: &str) -> usize {
        self.sessions
            .iter()
            .filter(|entry| entry.value().account_id == account_id)
            .count()
    }
}

impl ProxySessionGuard {
    pub(crate) fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown.clone()
    }
}

impl Drop for ProxySessionGuard {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            self.sessions.remove(id);
        }
    }
}
