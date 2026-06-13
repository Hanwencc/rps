mod bridge;
mod db;
mod policy;
mod proxy_http;
mod proxy_manager;
mod proxy_socks5;
mod proxy_tcp;
mod proxy_udp;
mod traffic;
mod tunnel_manager;
mod web;

use anyhow::Context;
use clap::Parser;
use dashmap::DashMap;
use db::Database;
use policy::PolicyEnforcer;
use proxy_manager::ProxyManager;
use rps_core::config::{ControllerConfig, load_controller_config};
use rps_mux::MuxHandle;
use std::sync::Arc;
use tracing::info;
use traffic::TrafficAggregator;
use tunnel_manager::TunnelManager;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "configs/controller.toml")]
    config: String,
}

#[derive(Clone)]
pub(crate) struct AppState {
    config: Arc<ControllerConfig>,
    db: Database,
    traffic: TrafficAggregator,
    clients: Arc<DashMap<String, MuxHandle>>,
    web_sessions: Arc<DashMap<String, WebSession>>,
    tunnel_manager: Arc<TunnelManager>,
    proxy_manager: ProxyManager,
    policy: PolicyEnforcer,
}

#[derive(Clone)]
pub(crate) struct WebSession {
    username: String,
    expires_at: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let config = Arc::new(load_controller_config(&args.config)?);
    let db = Database::open(&config.server.database_path, &config).await?;
    let (traffic, traffic_rx) = TrafficAggregator::channel();
    let (policy, policy_rx) = PolicyEnforcer::channel();
    let state = AppState {
        config: config.clone(),
        db: db.clone(),
        traffic,
        clients: Arc::new(DashMap::new()),
        web_sessions: Arc::new(DashMap::new()),
        tunnel_manager: Arc::new(TunnelManager::new()),
        proxy_manager: ProxyManager::new(),
        policy,
    };

    info!(bridge_addr = %config.server.bridge_addr, "starting rps-controller");
    state.policy.load_from_db(&state).await?;
    tokio::spawn(traffic::run(db, traffic_rx));
    tokio::spawn(policy::run(state.clone(), policy_rx));
    tokio::spawn(capture_usage_snapshots(state.clone()));
    tokio::spawn(bridge::run(state.clone()));
    tokio::spawn(web::run(state.clone()));

    state.proxy_manager.start_from_config(state.clone());

    state
        .tunnel_manager
        .start_enabled_from_db(state.clone())
        .await?;

    tokio::signal::ctrl_c()
        .await
        .context("failed to wait for ctrl-c")?;
    Ok(())
}

async fn capture_usage_snapshots(state: AppState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        if let Err(err) = state.db.capture_usage_snapshot().await {
            tracing::warn!(error = %err, "failed to capture usage snapshot");
        }
    }
}
