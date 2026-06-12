mod bridge;
mod db;
mod proxy_http;
mod proxy_socks5;
mod proxy_tcp;
mod proxy_udp;
mod web;

use anyhow::Context;
use clap::Parser;
use dashmap::DashMap;
use db::Database;
use rps_core::config::{ControllerConfig, TunnelMode, load_controller_config};
use rps_mux::MuxHandle;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "configs/controller.toml")]
    config: String,
}

#[derive(Clone)]
pub(crate) struct AppState {
    config: Arc<ControllerConfig>,
    db: Database,
    clients: Arc<DashMap<String, MuxHandle>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let config = Arc::new(load_controller_config(&args.config)?);
    let db = Database::open(&config.server.database_path, &config).await?;
    let state = AppState {
        config: config.clone(),
        db,
        clients: Arc::new(DashMap::new()),
    };

    info!(bridge_addr = %config.server.bridge_addr, "starting rps-controller");
    tokio::spawn(capture_usage_snapshots(state.clone()));
    tokio::spawn(bridge::run(state.clone()));
    tokio::spawn(web::run(state.clone()));

    if let Some(proxy) = config.server.http_proxy.clone().filter(|p| p.enabled) {
        tokio::spawn(proxy_http::run(state.clone(), proxy));
    }

    if let Some(proxy) = config.server.socks5.clone().filter(|p| p.enabled) {
        tokio::spawn(proxy_socks5::run(state.clone(), proxy));
    }

    for tunnel in config.tunnels.iter().filter(|t| t.enabled).cloned() {
        let state = state.clone();
        match tunnel.mode {
            TunnelMode::Tcp => {
                tokio::spawn(proxy_tcp::run(state, tunnel));
            }
            TunnelMode::Udp => {
                tokio::spawn(proxy_udp::run(state, tunnel));
            }
        }
    }

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
