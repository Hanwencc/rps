use crate::AppState;
use dashmap::DashMap;
use std::{
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const REVOKE_QUEUE_CAPACITY: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ResourceKind {
    Tunnel,
    ProxyAccount,
}

#[derive(Debug, Clone, Eq)]
pub(crate) struct ResourceKey {
    pub kind: ResourceKind,
    pub id: String,
}

impl PartialEq for ResourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}

impl Hash for ResourceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.id.hash(state);
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RevokeReason {
    Expired,
    TrafficExhausted,
}

impl RevokeReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::TrafficExhausted => "traffic_exhausted",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PolicyEnforcer {
    policies: Arc<DashMap<ResourceKey, Arc<PolicyState>>>,
    revoke_tx: mpsc::Sender<RevokeEvent>,
}

struct PolicyState {
    expires_at: Option<i64>,
    traffic_limit_bytes: Option<u64>,
    used_bytes: AtomicU64,
    revoked: AtomicBool,
}

#[derive(Debug)]
pub(crate) struct RevokeEvent {
    key: ResourceKey,
    reason: RevokeReason,
}

impl PolicyEnforcer {
    pub(crate) fn channel() -> (Self, mpsc::Receiver<RevokeEvent>) {
        let (revoke_tx, revoke_rx) = mpsc::channel(REVOKE_QUEUE_CAPACITY);
        (
            Self {
                policies: Arc::new(DashMap::new()),
                revoke_tx,
            },
            revoke_rx,
        )
    }

    pub(crate) async fn load_from_db(&self, state: &AppState) -> anyhow::Result<()> {
        for tunnel in state.db.list_tunnels().await? {
            self.register(
                ResourceKey {
                    kind: ResourceKind::Tunnel,
                    id: tunnel.id,
                },
                tunnel.expires_at,
                tunnel.traffic_limit_bytes,
                tunnel.rx_bytes.saturating_add(tunnel.tx_bytes),
            );
        }

        for account in state.db.list_proxy_accounts(None).await? {
            self.register(
                ResourceKey {
                    kind: ResourceKind::ProxyAccount,
                    id: account.id,
                },
                account.expires_at,
                account.traffic_limit_bytes,
                account.rx_bytes.saturating_add(account.tx_bytes),
            );
        }
        Ok(())
    }

    pub(crate) fn register(
        &self,
        key: ResourceKey,
        expires_at: Option<i64>,
        traffic_limit_bytes: Option<u64>,
        used_bytes: u64,
    ) {
        self.policies.insert(
            key,
            Arc::new(PolicyState {
                expires_at,
                traffic_limit_bytes,
                used_bytes: AtomicU64::new(used_bytes),
                revoked: AtomicBool::new(false),
            }),
        );
    }

    pub(crate) fn remove(&self, key: &ResourceKey) {
        self.policies.remove(key);
    }

    pub(crate) fn allowed(&self, key: &ResourceKey) -> bool {
        let Some(policy) = self.policies.get(key) else {
            return true;
        };
        if policy.revoked.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(expires_at) = policy.expires_at {
            if now_secs() >= expires_at {
                self.revoke_once(key.clone(), &policy, RevokeReason::Expired);
                return false;
            }
        }
        if let Some(limit) = policy.traffic_limit_bytes {
            if policy.used_bytes.load(Ordering::Relaxed) >= limit {
                self.revoke_once(key.clone(), &policy, RevokeReason::TrafficExhausted);
                return false;
            }
        }
        true
    }

    pub(crate) fn record_route_usage(&self, route_id: &str, rx_bytes: u64, tx_bytes: u64) {
        let Some(key) = resource_key_from_route(route_id) else {
            return;
        };
        let Some(policy) = self.policies.get(&key) else {
            return;
        };
        let delta = rx_bytes.saturating_add(tx_bytes);
        if delta == 0 {
            return;
        }
        let used = policy
            .used_bytes
            .fetch_add(delta, Ordering::Relaxed)
            .saturating_add(delta);
        if let Some(limit) = policy.traffic_limit_bytes {
            if used >= limit {
                self.revoke_once(key, &policy, RevokeReason::TrafficExhausted);
            }
        }
    }

    fn revoke_once(&self, key: ResourceKey, policy: &PolicyState, reason: RevokeReason) {
        if policy
            .revoked
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            if let Err(err) = self.revoke_tx.try_send(RevokeEvent { key, reason }) {
                debug!(error = %err, "policy revoke queue full");
            }
        }
    }

    fn expired_keys(&self) -> Vec<(ResourceKey, RevokeReason)> {
        let now = now_secs();
        self.policies
            .iter()
            .filter_map(|entry| {
                let policy = entry.value();
                policy.expires_at.and_then(|expires_at| {
                    (now >= expires_at && !policy.revoked.load(Ordering::Relaxed)).then(|| {
                        let key = entry.key().clone();
                        self.revoke_once(key.clone(), policy, RevokeReason::Expired);
                        (key, RevokeReason::Expired)
                    })
                })
            })
            .collect()
    }
}

pub(crate) async fn run(state: AppState, mut revoke_rx: mpsc::Receiver<RevokeEvent>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            Some(event) = revoke_rx.recv() => {
                revoke(&state, event).await;
            }
            _ = interval.tick() => {
                for (key, reason) in state.policy.expired_keys() {
                    revoke(&state, RevokeEvent { key, reason }).await;
                }
            }
        }
    }
}

async fn revoke(state: &AppState, event: RevokeEvent) {
    match event.key.kind {
        ResourceKind::Tunnel => {
            if let Err(err) = state
                .db
                .disable_tunnel(&event.key.id, event.reason.as_str())
                .await
            {
                warn!(resource_id = %event.key.id, error = %err, "failed to disable tunnel");
            }
            if let Err(err) = state.tunnel_manager.stop(&event.key.id).await {
                warn!(resource_id = %event.key.id, error = %err, "failed to stop tunnel after policy revoke");
            }
        }
        ResourceKind::ProxyAccount => {
            if let Err(err) = state
                .db
                .disable_proxy_account(&event.key.id, event.reason.as_str())
                .await
            {
                warn!(resource_id = %event.key.id, error = %err, "failed to disable proxy account");
            }
            state.proxy_manager.revoke_account(&event.key.id);
        }
    }
}

pub(crate) fn tunnel_key(id: impl Into<String>) -> ResourceKey {
    ResourceKey {
        kind: ResourceKind::Tunnel,
        id: id.into(),
    }
}

pub(crate) fn proxy_account_key(id: impl Into<String>) -> ResourceKey {
    ResourceKey {
        kind: ResourceKind::ProxyAccount,
        id: id.into(),
    }
}

fn resource_key_from_route(route_id: &str) -> Option<ResourceKey> {
    route_id
        .strip_prefix("http-proxy:")
        .or_else(|| route_id.strip_prefix("socks5:"))
        .or_else(|| route_id.strip_prefix("socks5-udp:"))
        .map(proxy_account_key)
        .or_else(|| Some(tunnel_key(route_id)))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
