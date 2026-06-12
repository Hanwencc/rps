use crate::AppState;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use rps_core::config::{ProxyListenConfig, TunnelMode};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Serialize)]
struct StatusResponse {
    bridge_addr: String,
    web_addr: String,
    online_clients: usize,
    configured_clients: usize,
    enabled_tunnels: usize,
    http_proxy_enabled: bool,
    socks5_enabled: bool,
}

#[derive(Debug, Serialize)]
struct ClientResponse {
    id: String,
    vkey: String,
    enabled: bool,
    online: bool,
    remark: Option<String>,
    max_connections: Option<u32>,
    compress: bool,
    encrypt: bool,
}

#[derive(Debug, Deserialize)]
struct CreateClientRequest {
    vkey: Option<String>,
    enabled: Option<bool>,
    remark: Option<String>,
    max_connections: Option<u32>,
    #[serde(default)]
    compress: bool,
    #[serde(default)]
    encrypt: bool,
}

#[derive(Debug, Serialize)]
struct TunnelResponse {
    id: String,
    client_id: String,
    mode: TunnelMode,
    listen: String,
    target: Option<String>,
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct ProxyResponse {
    http_proxy: Option<ProxyListenConfig>,
    socks5: Option<ProxyListenConfig>,
}

#[derive(Debug, Serialize)]
struct ProxyAccountResponse {
    id: String,
    kind: String,
    client_id: String,
    username: String,
    password: String,
    enabled: bool,
    remark: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProxyAccountsQuery {
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateProxyAccountRequest {
    kind: String,
    client_id: String,
    username: Option<String>,
    password: Option<String>,
    enabled: Option<bool>,
    remark: Option<String>,
}

pub async fn run(state: AppState) {
    if let Err(err) = run_inner(state).await {
        error!(error = %err, "web console stopped");
    }
}

async fn run_inner(state: AppState) -> anyhow::Result<()> {
    let web_addr: SocketAddr = state.config.server.web_addr.parse()?;
    let web_dir = PathBuf::from(&state.config.server.web_dir);
    let index = web_dir.join("index.html");

    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/clients", get(clients).post(create_client))
        .route("/api/tunnels", get(tunnels))
        .route("/api/proxy", get(proxy))
        .route(
            "/api/proxy-accounts",
            get(proxy_accounts).post(create_proxy_account),
        )
        .fallback_service(ServeDir::new(&web_dir).fallback(ServeFile::new(index)))
        .with_state(state);

    info!(listen = %web_addr, web_dir = %web_dir.display(), "web console listening");
    let listener = tokio::net::TcpListener::bind(web_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn status(State(state): State<AppState>) -> Result<Json<StatusResponse>, ApiError> {
    let configured_clients = state.db.count_clients().await?;
    let enabled_tunnels = state.db.count_enabled_tunnels().await?;
    Ok(Json(StatusResponse {
        bridge_addr: state.config.server.bridge_addr.clone(),
        web_addr: state.config.server.web_addr.clone(),
        online_clients: state.clients.len(),
        configured_clients,
        enabled_tunnels,
        http_proxy_enabled: state
            .config
            .server
            .http_proxy
            .as_ref()
            .is_some_and(|p| p.enabled),
        socks5_enabled: state
            .config
            .server
            .socks5
            .as_ref()
            .is_some_and(|p| p.enabled),
    }))
}

async fn clients(State(state): State<AppState>) -> Result<Json<Vec<ClientResponse>>, ApiError> {
    let clients = state.db.list_clients().await?;
    Ok(Json(
        clients
            .into_iter()
            .map(|client| client_response(&state, client))
            .collect(),
    ))
}

async fn create_client(
    State(state): State<AppState>,
    Json(request): Json<CreateClientRequest>,
) -> Result<(StatusCode, Json<ClientResponse>), ApiError> {
    let id = Uuid::new_v4().to_string();
    let vkey = request
        .vkey
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let client = state
        .db
        .create_client(crate::db::NewClient {
            id,
            vkey,
            enabled: request.enabled.unwrap_or(true),
            remark: request.remark.filter(|value| !value.trim().is_empty()),
            max_connections: request.max_connections,
            compress: request.compress,
            encrypt: request.encrypt,
        })
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(client_response(&state, client))))
}

async fn tunnels(State(state): State<AppState>) -> Result<Json<Vec<TunnelResponse>>, ApiError> {
    let tunnels = state.db.list_tunnels().await?;
    Ok(Json(
        tunnels
            .into_iter()
            .map(|tunnel| TunnelResponse {
                id: tunnel.id,
                client_id: tunnel.client_id,
                mode: tunnel.mode,
                listen: tunnel.listen,
                target: tunnel.target,
                enabled: tunnel.enabled,
            })
            .collect(),
    ))
}

async fn proxy(State(state): State<AppState>) -> Result<Json<ProxyResponse>, ApiError> {
    Ok(Json(ProxyResponse {
        http_proxy: state.db.get_proxy("http").await?,
        socks5: state.db.get_proxy("socks5").await?,
    }))
}

async fn proxy_accounts(
    State(state): State<AppState>,
    Query(query): Query<ProxyAccountsQuery>,
) -> Result<Json<Vec<ProxyAccountResponse>>, ApiError> {
    let accounts = state.db.list_proxy_accounts(query.kind.as_deref()).await?;
    Ok(Json(
        accounts.into_iter().map(proxy_account_response).collect(),
    ))
}

async fn create_proxy_account(
    State(state): State<AppState>,
    Json(request): Json<CreateProxyAccountRequest>,
) -> Result<(StatusCode, Json<ProxyAccountResponse>), ApiError> {
    if state.db.get_client(&request.client_id).await?.is_none() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("client {} not found", request.client_id),
        });
    }
    let account = state
        .db
        .create_proxy_account(crate::db::NewProxyAccount {
            id: Uuid::new_v4().to_string(),
            kind: request.kind,
            client_id: request.client_id,
            username: request
                .username
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(random_proxy_secret),
            password: request
                .password
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(random_proxy_secret),
            enabled: request.enabled.unwrap_or(true),
            remark: request.remark.filter(|value| !value.trim().is_empty()),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(proxy_account_response(account))))
}

fn client_response(state: &AppState, client: crate::db::DbClient) -> ClientResponse {
    ClientResponse {
        id: client.id.clone(),
        vkey: client.vkey,
        enabled: client.enabled,
        online: state.clients.contains_key(&client.id),
        remark: client.remark,
        max_connections: client.max_connections,
        compress: client.compress,
        encrypt: client.encrypt,
    }
}

fn proxy_account_response(account: crate::db::DbProxyAccount) -> ProxyAccountResponse {
    ProxyAccountResponse {
        id: account.id,
        kind: account.kind,
        client_id: account.client_id,
        username: account.username,
        password: account.password,
        enabled: account.enabled,
        remark: account.remark,
    }
}

fn random_proxy_secret() -> String {
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(12)
        .collect()
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        let message = err.to_string();
        let status = if message.contains("UNIQUE constraint failed") {
            StatusCode::CONFLICT
        } else if message.contains("invalid proxy account kind") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, message }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, self.message).into_response()
    }
}
