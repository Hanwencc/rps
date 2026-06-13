use crate::{AppState, WebSession, tunnel_manager::tunnel_config_from_db};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::sse::{Event, KeepAlive, Sse},
    routing::{delete, get, post},
};
use hmac::{Hmac, Mac};
use rps_core::config::{ProxyListenConfig, TunnelMode};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::{convert::Infallible, net::SocketAddr, path::PathBuf, time::Duration};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info};
use uuid::Uuid;

const SESSION_COOKIE: &str = "rps_session";
const TOTP_STEP_SECS: i64 = 30;
const TOTP_DIGITS: u32 = 6;
const EVENTS_INTERVAL: Duration = Duration::from_secs(1);
type HmacSha1 = Hmac<Sha1>;

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
    psk: String,
    enabled: bool,
    online: bool,
    remark: Option<String>,
    max_connections: Option<u32>,
    compress: bool,
    encrypt: bool,
    rx_bytes: u64,
    tx_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct CreateClientRequest {
    psk: Option<String>,
    enabled: Option<bool>,
    remark: Option<String>,
    max_connections: Option<u32>,
    #[serde(default)]
    compress: bool,
    #[serde(default)]
    encrypt: bool,
}

#[derive(Debug, Deserialize)]
struct CreateTunnelRequest {
    id: Option<String>,
    client_id: String,
    mode: TunnelMode,
    listen: String,
    target: Option<String>,
    enabled: Option<bool>,
    expires_at: Option<i64>,
    traffic_limit_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
struct TunnelResponse {
    id: String,
    client_id: String,
    mode: TunnelMode,
    listen: String,
    target: Option<String>,
    enabled: bool,
    active_connections: usize,
    expires_at: Option<i64>,
    traffic_limit_bytes: Option<u64>,
    rx_bytes: u64,
    tx_bytes: u64,
    disabled_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProxyResponse {
    http_proxy: Option<ProxyListenConfig>,
    socks5: Option<ProxyListenConfig>,
}

#[derive(Debug, Serialize)]
struct ConsoleDataResponse {
    status: StatusResponse,
    clients: Vec<ClientResponse>,
    tunnels: Vec<TunnelResponse>,
    proxy: ProxyResponse,
    #[serde(rename = "proxyAccounts")]
    proxy_accounts: Vec<ProxyAccountResponse>,
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
    active_connections: usize,
    expires_at: Option<i64>,
    traffic_limit_bytes: Option<u64>,
    rx_bytes: u64,
    tx_bytes: u64,
    disabled_reason: Option<String>,
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
    expires_at: Option<i64>,
    traffic_limit_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    otp_code: Option<String>,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    authenticated: bool,
    requires_2fa: bool,
    username: Option<String>,
    security_key_available: bool,
}

#[derive(Debug, Serialize)]
struct AuthStatusResponse {
    authenticated: bool,
    username: Option<String>,
    two_factor_enabled: bool,
    security_key_available: bool,
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
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/events", get(events))
        .route("/api/status", get(status))
        .route("/api/clients", get(clients).post(create_client))
        .route("/api/clients/{id}", delete(delete_client))
        .route("/api/tunnels", get(tunnels).post(create_tunnel))
        .route("/api/tunnels/{id}", delete(delete_tunnel))
        .route("/api/proxy", get(proxy))
        .route(
            "/api/proxy-accounts",
            get(proxy_accounts).post(create_proxy_account),
        )
        .route("/api/proxy-accounts/{id}", delete(delete_proxy_account))
        .fallback_service(ServeDir::new(&web_dir).fallback(ServeFile::new(index)))
        .with_state(state);

    info!(listen = %web_addr, web_dir = %web_dir.display(), "web console listening");
    let listener = tokio::net::TcpListener::bind(web_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn auth_status(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<AuthStatusResponse>, ApiError> {
    let username = authenticated_username(&headers, &state);
    Ok(Json(AuthStatusResponse {
        authenticated: username.is_some() || !state.config.server.web_auth.enabled,
        username,
        two_factor_enabled: two_factor_enabled(&state),
        security_key_available: false,
    }))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<(HeaderMap, Json<LoginResponse>), ApiError> {
    let auth = &state.config.server.web_auth;
    if !auth.enabled {
        return Ok((
            HeaderMap::new(),
            Json(LoginResponse {
                authenticated: true,
                requires_2fa: false,
                username: Some(auth.username.clone()),
                security_key_available: false,
            }),
        ));
    }

    if request.username != auth.username || request.password != auth.password {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "用户名或密码错误".to_string(),
        });
    }

    if let Some(secret) = normalized_totp_secret(&state) {
        let Some(code) = request
            .otp_code
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        else {
            return Ok((
                HeaderMap::new(),
                Json(LoginResponse {
                    authenticated: false,
                    requires_2fa: true,
                    username: None,
                    security_key_available: false,
                }),
            ));
        };

        if !verify_totp(&secret, code)? {
            return Err(ApiError {
                status: StatusCode::UNAUTHORIZED,
                message: "动态验证码错误".to_string(),
            });
        }
    }

    let token = Uuid::new_v4().to_string();
    let ttl = auth.session_ttl_secs.max(60);
    let expires_at = now_secs() + ttl as i64;
    state.web_sessions.insert(
        token.clone(),
        WebSession {
            username: auth.username.clone(),
            expires_at,
        },
    );

    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, session_cookie(&token, ttl)?);
    Ok((
        headers,
        Json(LoginResponse {
            authenticated: true,
            requires_2fa: false,
            username: Some(auth.username.clone()),
            security_key_available: false,
        }),
    ))
}

async fn logout(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<(HeaderMap, Json<AuthStatusResponse>), ApiError> {
    if let Some(token) = session_token(&headers) {
        state.web_sessions.remove(&token);
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::SET_COOKIE, expired_session_cookie()?);
    Ok((
        response_headers,
        Json(AuthStatusResponse {
            authenticated: false,
            username: None,
            two_factor_enabled: two_factor_enabled(&state),
            security_key_available: false,
        }),
    ))
}

async fn status(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, ApiError> {
    require_auth(&headers, &state)?;
    Ok(Json(status_response(&state).await?))
}

async fn events(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&headers, &state)?;
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(EVENTS_INTERVAL);
        loop {
            interval.tick().await;
            if require_auth(&headers, &state).is_err() {
                yield Ok(Event::default().event("auth_expired").data("unauthorized"));
                break;
            }
            match console_data_response(&state).await {
                Ok(data) => match serde_json::to_string(&data) {
                    Ok(json) => yield Ok(Event::default().event("snapshot").data(json)),
                    Err(err) => yield Ok(Event::default().event("stream_error").data(err.to_string())),
                },
                Err(err) => yield Ok(Event::default().event("stream_error").data(err.to_string())),
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn clients(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<ClientResponse>>, ApiError> {
    require_auth(&headers, &state)?;
    let clients = state.db.list_clients().await?;
    let mut response = Vec::with_capacity(clients.len());
    for client in clients {
        response.push(client_response(&state, client).await?);
    }
    Ok(Json(response))
}

async fn create_client(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<CreateClientRequest>,
) -> Result<(StatusCode, Json<ClientResponse>), ApiError> {
    require_auth(&headers, &state)?;
    let id = Uuid::new_v4().to_string();
    let psk = request
        .psk
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(random_psk);
    let client = state
        .db
        .create_client(crate::db::NewClient {
            id,
            psk,
            enabled: request.enabled.unwrap_or(true),
            remark: request.remark.filter(|value| !value.trim().is_empty()),
            max_connections: request.max_connections,
            compress: request.compress,
            encrypt: request.encrypt,
        })
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::CREATED,
        Json(client_response(&state, client).await?),
    ))
}

async fn delete_client(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&headers, &state)?;
    if state.clients.contains_key(&id) {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: "client is online, stop agent before deleting it".to_string(),
        });
    }
    let reference_count = state.db.client_reference_count(&id).await?;
    if reference_count > 0 {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: format!(
                "client is still referenced by {reference_count} tunnel/proxy config(s)"
            ),
        });
    }
    if !state.db.delete_client(&id).await? {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("client {id} not found"),
        });
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn tunnels(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<TunnelResponse>>, ApiError> {
    require_auth(&headers, &state)?;
    let tunnels = state.db.list_tunnels().await?;
    let mut response = Vec::with_capacity(tunnels.len());
    for tunnel in tunnels {
        response.push(tunnel_response(&state, tunnel).await);
    }
    Ok(Json(response))
}

async fn create_tunnel(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<CreateTunnelRequest>,
) -> Result<(StatusCode, Json<TunnelResponse>), ApiError> {
    require_auth(&headers, &state)?;
    if state.db.get_client(&request.client_id).await?.is_none() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("client {} not found", request.client_id),
        });
    }
    let target = request
        .target
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "target is required".to_string(),
        })?;
    let tunnel = state
        .db
        .create_tunnel(crate::db::NewTunnel {
            id: request
                .id
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            client_id: request.client_id,
            mode: request.mode,
            listen: request.listen,
            target: Some(target),
            enabled: request.enabled.unwrap_or(true),
            expires_at: request.expires_at,
            traffic_limit_bytes: request.traffic_limit_bytes,
        })
        .await?;
    state.policy.register(
        crate::policy::tunnel_key(tunnel.id.clone()),
        tunnel.expires_at,
        tunnel.traffic_limit_bytes,
        tunnel.rx_bytes.saturating_add(tunnel.tx_bytes),
    );

    if tunnel.enabled {
        let tunnel_config = tunnel_config_from_db(&tunnel);
        if let Err(err) = state
            .tunnel_manager
            .start(state.clone(), tunnel_config)
            .await
        {
            let _ = state.db.delete_tunnel(&tunnel.id).await;
            state
                .policy
                .remove(&crate::policy::tunnel_key(tunnel.id.clone()));
            return Err(ApiError::from(err));
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(tunnel_response(&state, tunnel).await),
    ))
}

async fn delete_tunnel(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&headers, &state)?;
    if !state.db.delete_tunnel(&id).await? {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("tunnel {id} not found"),
        });
    }
    state.tunnel_manager.stop(&id).await?;
    state.policy.remove(&crate::policy::tunnel_key(id));
    Ok(StatusCode::NO_CONTENT)
}

async fn proxy(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<ProxyResponse>, ApiError> {
    require_auth(&headers, &state)?;
    Ok(Json(ProxyResponse {
        http_proxy: state.db.get_proxy("http").await?,
        socks5: state.db.get_proxy("socks5").await?,
    }))
}

async fn proxy_accounts(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<ProxyAccountsQuery>,
) -> Result<Json<Vec<ProxyAccountResponse>>, ApiError> {
    require_auth(&headers, &state)?;
    let accounts = state.db.list_proxy_accounts(query.kind.as_deref()).await?;
    Ok(Json(
        accounts
            .into_iter()
            .map(|account| proxy_account_response(&state, account))
            .collect(),
    ))
}

async fn status_response(state: &AppState) -> anyhow::Result<StatusResponse> {
    let configured_clients = state.db.count_clients().await?;
    let enabled_tunnels = state.db.count_enabled_tunnels().await?;
    Ok(StatusResponse {
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
    })
}

async fn console_data_response(state: &AppState) -> anyhow::Result<ConsoleDataResponse> {
    let clients = state.db.list_clients().await?;
    let mut client_responses = Vec::with_capacity(clients.len());
    for client in clients {
        client_responses.push(client_response(state, client).await?);
    }

    let tunnels = state.db.list_tunnels().await?;
    let mut tunnel_responses = Vec::with_capacity(tunnels.len());
    for tunnel in tunnels {
        tunnel_responses.push(tunnel_response(state, tunnel).await);
    }

    let proxy_accounts = state
        .db
        .list_proxy_accounts(None)
        .await?
        .into_iter()
        .map(|account| proxy_account_response(state, account))
        .collect();

    Ok(ConsoleDataResponse {
        status: status_response(state).await?,
        clients: client_responses,
        tunnels: tunnel_responses,
        proxy: ProxyResponse {
            http_proxy: state.db.get_proxy("http").await?,
            socks5: state.db.get_proxy("socks5").await?,
        },
        proxy_accounts,
    })
}

async fn create_proxy_account(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<CreateProxyAccountRequest>,
) -> Result<(StatusCode, Json<ProxyAccountResponse>), ApiError> {
    require_auth(&headers, &state)?;
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
            expires_at: request.expires_at,
            traffic_limit_bytes: request.traffic_limit_bytes,
        })
        .await?;
    state.policy.register(
        crate::policy::proxy_account_key(account.id.clone()),
        account.expires_at,
        account.traffic_limit_bytes,
        account.rx_bytes.saturating_add(account.tx_bytes),
    );
    Ok((
        StatusCode::CREATED,
        Json(proxy_account_response(&state, account)),
    ))
}

async fn delete_proxy_account(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&headers, &state)?;
    if !state.db.delete_proxy_account(&id).await? {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("proxy account {id} not found"),
        });
    }
    state.proxy_manager.revoke_account(&id);
    state.policy.remove(&crate::policy::proxy_account_key(id));
    Ok(StatusCode::NO_CONTENT)
}

async fn tunnel_response(state: &AppState, tunnel: crate::db::DbTunnel) -> TunnelResponse {
    let active_connections = state.tunnel_manager.active_count(&tunnel.id).await;
    TunnelResponse {
        id: tunnel.id,
        client_id: tunnel.client_id,
        mode: tunnel.mode,
        listen: tunnel.listen,
        target: tunnel.target,
        enabled: tunnel.enabled,
        active_connections,
        expires_at: tunnel.expires_at,
        traffic_limit_bytes: tunnel.traffic_limit_bytes,
        rx_bytes: tunnel.rx_bytes,
        tx_bytes: tunnel.tx_bytes,
        disabled_reason: tunnel.disabled_reason,
    }
}

async fn client_response(
    state: &AppState,
    client: crate::db::DbClient,
) -> anyhow::Result<ClientResponse> {
    let traffic = state.db.get_traffic_counter("client", &client.id).await?;
    Ok(ClientResponse {
        id: client.id.clone(),
        psk: client.psk,
        enabled: client.enabled,
        online: state.clients.contains_key(&client.id),
        remark: client.remark,
        max_connections: client.max_connections,
        compress: client.compress,
        encrypt: client.encrypt,
        rx_bytes: traffic.rx_bytes,
        tx_bytes: traffic.tx_bytes,
    })
}

fn proxy_account_response(
    state: &AppState,
    account: crate::db::DbProxyAccount,
) -> ProxyAccountResponse {
    let active_connections = state.proxy_manager.active_count(&account.id);
    ProxyAccountResponse {
        id: account.id,
        kind: account.kind,
        client_id: account.client_id,
        username: account.username,
        password: account.password,
        enabled: account.enabled,
        remark: account.remark,
        active_connections,
        expires_at: account.expires_at,
        traffic_limit_bytes: account.traffic_limit_bytes,
        rx_bytes: account.rx_bytes,
        tx_bytes: account.tx_bytes,
        disabled_reason: account.disabled_reason,
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

fn random_psk() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn require_auth(headers: &HeaderMap, state: &AppState) -> Result<(), ApiError> {
    if !state.config.server.web_auth.enabled {
        return Ok(());
    }
    if authenticated_username(headers, state).is_some() {
        return Ok(());
    }
    Err(ApiError {
        status: StatusCode::UNAUTHORIZED,
        message: "unauthorized".to_string(),
    })
}

fn authenticated_username(headers: &HeaderMap, state: &AppState) -> Option<String> {
    let token = session_token(headers)?;
    let session = state.web_sessions.get(&token)?;
    if session.expires_at < now_secs() {
        drop(session);
        state.web_sessions.remove(&token);
        return None;
    }
    Some(session.username.clone())
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    for cookie in cookies.split(';') {
        let (name, value) = cookie.trim().split_once('=')?;
        if name == SESSION_COOKIE && !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn session_cookie(token: &str, ttl_secs: u64) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; Path=/; Max-Age={ttl_secs}; HttpOnly; SameSite=Lax"
    ))
    .map_err(|err| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: err.to_string(),
    })
}

fn expired_session_cookie() -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
    ))
    .map_err(|err| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: err.to_string(),
    })
}

fn two_factor_enabled(state: &AppState) -> bool {
    normalized_totp_secret(state).is_some()
}

fn normalized_totp_secret(state: &AppState) -> Option<String> {
    state
        .config
        .server
        .web_auth
        .totp_secret
        .as_ref()
        .map(|secret| secret.trim().replace(' ', ""))
        .filter(|secret| !secret.is_empty())
}

fn verify_totp(secret: &str, code: &str) -> Result<bool, ApiError> {
    let secret = decode_base32(secret)?;
    let code = code.trim();
    if code.len() != TOTP_DIGITS as usize || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(false);
    }

    let now_step = now_secs() / TOTP_STEP_SECS;
    for step in [now_step - 1, now_step, now_step + 1] {
        if totp_at_step(&secret, step as u64)? == code {
            return Ok(true);
        }
    }
    Ok(false)
}

fn totp_at_step(secret: &[u8], step: u64) -> Result<String, ApiError> {
    let mut mac = HmacSha1::new_from_slice(secret).map_err(|err| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: err.to_string(),
    })?;
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let value = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let modulo = 10_u32.pow(TOTP_DIGITS);
    Ok(format!("{:06}", value % modulo))
}

fn decode_base32(input: &str) -> Result<Vec<u8>, ApiError> {
    let mut bits = 0_u32;
    let mut bit_count = 0_u8;
    let mut out = Vec::new();

    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => {
                return Err(ApiError {
                    status: StatusCode::BAD_REQUEST,
                    message: "invalid totp secret".to_string(),
                });
            }
        };
        bits = (bits << 5) | u32::from(value);
        bit_count += 5;
        while bit_count >= 8 {
            bit_count -= 8;
            out.push(((bits >> bit_count) & 0xff) as u8);
        }
    }

    if out.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "invalid totp secret".to_string(),
        });
    }
    Ok(out)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
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
