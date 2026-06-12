use crate::{AppState, proxy_tcp};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use rps_core::{config::ProxyListenConfig, protocol::TargetProtocol};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{error, info, warn};

const MAX_HEADER: usize = 64 * 1024;

pub async fn run(state: AppState, proxy: ProxyListenConfig) {
    if let Err(err) = run_inner(state, proxy).await {
        error!(error = %err, "http proxy stopped");
    }
}

async fn run_inner(state: AppState, proxy: ProxyListenConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&proxy.listen).await?;
    info!(client_id = %proxy.client_id, listen = %proxy.listen, "http proxy listening");
    loop {
        let (socket, remote_addr) = listener.accept().await?;
        let state = state.clone();
        let route = proxy_route(&proxy);
        tokio::spawn(async move {
            if let Err(err) = handle_http_proxy(state, route, socket, remote_addr.to_string()).await
            {
                warn!(error = %err, "http proxy connection failed");
            }
        });
    }
}

async fn handle_http_proxy(
    state: AppState,
    route: proxy_tcp::StreamRoute,
    mut socket: TcpStream,
    remote_addr: String,
) -> anyhow::Result<()> {
    let header = read_header(&mut socket).await?;
    let header_text = std::str::from_utf8(&header)?;
    let route = match authenticated_route(&state, &route, header_text).await? {
        Some(route) => route,
        None => {
            socket
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"rps\"\r\nContent-Length: 0\r\n\r\n",
                )
                .await?;
            anyhow::bail!("http proxy authentication failed");
        }
    };
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty http request"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let uri = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or("HTTP/1.1");

    if method.eq_ignore_ascii_case("CONNECT") {
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let recorder = proxy_tcp::TrafficRecorder::new(&state, &route);
        let stream = proxy_tcp::open_stream(
            state,
            &route,
            TargetProtocol::Tcp,
            uri.to_string(),
            remote_addr,
        )
        .await?;
        return proxy_tcp::pipe_tcp_mux(socket, stream, None, Some(recorder)).await;
    }

    let target = http_target(uri, header_text)?;
    let rewritten = rewrite_absolute_form(method, uri, version, header_text);
    let recorder = proxy_tcp::TrafficRecorder::new(&state, &route);
    let stream =
        proxy_tcp::open_stream(state, &route, TargetProtocol::Tcp, target, remote_addr).await?;
    proxy_tcp::pipe_tcp_mux(socket, stream, Some(Bytes::from(rewritten)), Some(recorder)).await
}

async fn authenticated_route(
    state: &AppState,
    fallback: &proxy_tcp::StreamRoute,
    header: &str,
) -> anyhow::Result<Option<proxy_tcp::StreamRoute>> {
    if !state.db.has_enabled_proxy_accounts("http").await? {
        return Ok(Some(fallback.clone()));
    }
    let Some((username, password)) = parse_basic_proxy_auth(header) else {
        return Ok(None);
    };
    let Some(account) = state
        .db
        .find_enabled_proxy_account("http", &username, &password)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(proxy_tcp::StreamRoute {
        tunnel_id: format!("http-proxy:{}", account.id),
        client_id: account.client_id,
    }))
}

fn parse_basic_proxy_auth(header: &str) -> Option<(String, String)> {
    for line in header.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("Proxy-Authorization") {
            continue;
        }
        let encoded = value.trim().strip_prefix("Basic ")?;
        let decoded = BASE64.decode(encoded).ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let (username, password) = decoded.split_once(':')?;
        return Some((username.to_string(), password.to_string()));
    }
    None
}

async fn read_header(socket: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut header = Vec::with_capacity(4096);
    let mut buf = [0_u8; 1024];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            anyhow::bail!("connection closed before http header");
        }
        header.extend_from_slice(&buf[..n]);
        if header.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(header);
        }
        if header.len() > MAX_HEADER {
            anyhow::bail!("http header too large");
        }
    }
}

fn http_target(uri: &str, header: &str) -> anyhow::Result<String> {
    if let Some(rest) = uri.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or_default();
        return Ok(with_default_port(host, 80));
    }
    if let Some(rest) = uri.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or_default();
        return Ok(with_default_port(host, 443));
    }
    for line in header.lines() {
        if let Some(host) = line.strip_prefix("Host:") {
            return Ok(with_default_port(host.trim(), 80));
        }
        if let Some(host) = line.strip_prefix("host:") {
            return Ok(with_default_port(host.trim(), 80));
        }
    }
    anyhow::bail!("http proxy target not found")
}

fn with_default_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

fn rewrite_absolute_form(method: &str, uri: &str, version: &str, header: &str) -> Vec<u8> {
    let path = if let Some(rest) = uri.strip_prefix("http://") {
        let idx = rest.find('/').unwrap_or(rest.len());
        &rest[idx..]
    } else if let Some(rest) = uri.strip_prefix("https://") {
        let idx = rest.find('/').unwrap_or(rest.len());
        &rest[idx..]
    } else {
        uri
    };
    let path = if path.is_empty() { "/" } else { path };
    let rest = header
        .split_once("\r\n")
        .map(|(_, rest)| rest)
        .unwrap_or("\r\n");
    let rest = strip_proxy_headers(rest);
    format!("{method} {path} {version}\r\n{rest}").into_bytes()
}

fn strip_proxy_headers(header: &str) -> String {
    header
        .split("\r\n")
        .filter(|line| {
            !line
                .split_once(':')
                .map(|(name, _)| name.eq_ignore_ascii_case("Proxy-Authorization"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn proxy_route(proxy: &ProxyListenConfig) -> proxy_tcp::StreamRoute {
    proxy_tcp::StreamRoute {
        tunnel_id: "http-proxy".to_string(),
        client_id: proxy.client_id.clone(),
    }
}
