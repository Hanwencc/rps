use crate::{AppState, proxy_tcp};
use bytes::Bytes;
use dashmap::DashMap;
use rps_core::{config::ProxyListenConfig, protocol::TargetProtocol};
use rps_mux::MuxStreamWriter;
use std::sync::atomic::AtomicUsize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tracing::{debug, error, info, warn};

const SOCKS_VERSION: u8 = 5;
const METHOD_NO_AUTH: u8 = 0;
const METHOD_USERNAME_PASSWORD: u8 = 2;
const CMD_CONNECT: u8 = 1;
const CMD_BIND: u8 = 2;
const CMD_UDP_ASSOCIATE: u8 = 3;

const ATYP_IPV4: u8 = 1;
const ATYP_DOMAIN: u8 = 3;
const ATYP_IPV6: u8 = 4;

const REP_SUCCEEDED: u8 = 0;
const REP_GENERAL_FAILURE: u8 = 1;
const REP_COMMAND_NOT_SUPPORTED: u8 = 7;
const REP_ADDR_TYPE_NOT_SUPPORTED: u8 = 8;

const UDP_IDLE_SECS: u64 = 120;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SocksUdpSessionKey {
    client_id: String,
    client_addr: SocketAddr,
    target: String,
}

struct SocksUdpSession {
    writer: MuxStreamWriter,
    last_seen: Arc<AtomicU64>,
}

struct SocksUdpAssociation {
    refs: AtomicUsize,
    last_seen: AtomicU64,
    client_id: String,
    udp_tunnel_id: String,
}

#[derive(Debug, Clone)]
struct SocksAuthContext {
    client_id: String,
    account_id: Option<String>,
}

#[derive(Debug, Clone)]
struct SocksAddr {
    host: String,
    port: u16,
    atyp: u8,
}

impl SocksAddr {
    fn target(&self) -> String {
        if self.atyp == ATYP_IPV6 {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    fn encode_udp_response(&self, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut packet = Vec::with_capacity(payload.len() + self.host.len() + 7);
        packet.extend_from_slice(&[0, 0, 0, self.atyp]);
        match self.atyp {
            ATYP_IPV4 => {
                let ip: Ipv4Addr = self.host.parse()?;
                packet.extend_from_slice(&ip.octets());
            }
            ATYP_IPV6 => {
                let ip: std::net::Ipv6Addr = self.host.parse()?;
                packet.extend_from_slice(&ip.octets());
            }
            ATYP_DOMAIN => {
                let host = self.host.as_bytes();
                anyhow::ensure!(host.len() <= u8::MAX as usize, "domain name too long");
                packet.push(host.len() as u8);
                packet.extend_from_slice(host);
            }
            _ => anyhow::bail!("unsupported socks address type {}", self.atyp),
        }
        packet.extend_from_slice(&self.port.to_be_bytes());
        packet.extend_from_slice(payload);
        Ok(packet)
    }
}

pub async fn run(state: AppState, proxy: ProxyListenConfig) {
    if let Err(err) = run_inner(state, proxy).await {
        error!(error = %err, "socks5 proxy stopped");
    }
}

async fn run_inner(state: AppState, proxy: ProxyListenConfig) -> anyhow::Result<()> {
    let tcp_listener = TcpListener::bind(&proxy.listen).await?;
    let udp_socket = Arc::new(UdpSocket::bind(&proxy.listen).await?);
    let udp_bind_addr = udp_socket.local_addr()?;
    let udp_addr = match proxy.public_udp_addr.as_deref() {
        Some(addr) => addr
            .parse::<SocketAddr>()
            .map_err(|err| anyhow::anyhow!("invalid socks5 public_udp_addr {addr}: {err}"))?,
        None => udp_bind_addr,
    };
    let associations = Arc::new(DashMap::<IpAddr, Arc<SocksUdpAssociation>>::new());
    let sessions = Arc::new(DashMap::<SocksUdpSessionKey, SocksUdpSession>::new());

    if proxy.public_udp_addr.is_none() && udp_addr.ip().is_unspecified() {
        warn!(
            udp_addr = %udp_addr,
            "socks5 udp associate will return an unspecified address; set public_udp_addr for public deployments"
        );
    }
    info!(listen = %proxy.listen, udp_bind_addr = %udp_bind_addr, udp_reply_addr = %udp_addr, "socks5 proxy listening");

    tokio::spawn(run_udp_relay(
        state.clone(),
        udp_socket,
        associations.clone(),
        sessions.clone(),
    ));
    tokio::spawn(cleanup_udp_state(associations.clone(), sessions.clone()));

    loop {
        let (socket, remote_addr) = tcp_listener.accept().await?;
        let state = state.clone();
        let proxy = proxy.clone();
        let associations = associations.clone();
        let sessions = sessions.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(
                state,
                proxy,
                socket,
                remote_addr,
                udp_addr,
                associations,
                sessions,
            )
            .await
            {
                warn!(%remote_addr, error = %err, "socks5 connection failed");
            }
        });
    }
}

async fn handle_conn(
    state: AppState,
    proxy: ProxyListenConfig,
    mut socket: TcpStream,
    remote_addr: SocketAddr,
    udp_addr: SocketAddr,
    associations: Arc<DashMap<IpAddr, Arc<SocksUdpAssociation>>>,
    sessions: Arc<DashMap<SocksUdpSessionKey, SocksUdpSession>>,
) -> anyhow::Result<()> {
    socket.set_nodelay(true)?;
    let mut hello = [0_u8; 2];
    socket.read_exact(&mut hello).await?;
    anyhow::ensure!(
        hello[0] == SOCKS_VERSION,
        "unsupported socks version {}",
        hello[0]
    );

    let mut methods = vec![0_u8; hello[1] as usize];
    socket.read_exact(&mut methods).await?;
    let auth = authenticate(&state, &proxy, &mut socket, &methods).await?;
    debug!(
        %remote_addr,
        account_id = ?auth.account_id,
        client_id = %auth.client_id,
        "socks5 authentication accepted"
    );

    let mut header = [0_u8; 4];
    socket.read_exact(&mut header).await?;
    anyhow::ensure!(
        header[0] == SOCKS_VERSION,
        "invalid request version {}",
        header[0]
    );
    anyhow::ensure!(header[2] == 0, "invalid reserved byte {}", header[2]);

    match header[1] {
        CMD_CONNECT => handle_connect(state, auth, socket, remote_addr, header[3]).await,
        CMD_UDP_ASSOCIATE => {
            handle_udp_associate(
                state,
                socket,
                remote_addr,
                udp_addr,
                header[3],
                auth,
                associations,
                sessions,
            )
            .await
        }
        CMD_BIND => {
            send_reply(&mut socket, REP_COMMAND_NOT_SUPPORTED, None).await?;
            Ok(())
        }
        command => {
            send_reply(&mut socket, REP_COMMAND_NOT_SUPPORTED, None).await?;
            anyhow::bail!("unsupported socks5 command {command}");
        }
    }
}

async fn authenticate(
    state: &AppState,
    proxy: &ProxyListenConfig,
    socket: &mut TcpStream,
    methods: &[u8],
) -> anyhow::Result<SocksAuthContext> {
    if state.db.has_enabled_proxy_accounts("socks5").await? {
        if !methods.contains(&METHOD_USERNAME_PASSWORD) {
            socket.write_all(&[SOCKS_VERSION, 0xff]).await?;
            anyhow::bail!("socks5 username/password method is not offered");
        }
        socket
            .write_all(&[SOCKS_VERSION, METHOD_USERNAME_PASSWORD])
            .await?;
        let (username, password) = read_username_password(socket).await?;
        let Some(account) = state
            .db
            .find_enabled_proxy_account("socks5", &username, &password)
            .await?
        else {
            socket.write_all(&[1, 1]).await?;
            anyhow::bail!("socks5 username/password authentication failed");
        };
        if !state
            .policy
            .allowed(&crate::policy::proxy_account_key(account.id.clone()))
        {
            socket.write_all(&[1, 1]).await?;
            anyhow::bail!("socks5 account disabled by policy");
        }
        socket.write_all(&[1, 0]).await?;
        return Ok(SocksAuthContext {
            client_id: account.client_id,
            account_id: Some(account.id),
        });
    }

    if !methods.contains(&METHOD_NO_AUTH) {
        socket.write_all(&[SOCKS_VERSION, 0xff]).await?;
        anyhow::bail!("socks5 no-auth method is not offered");
    }
    socket.write_all(&[SOCKS_VERSION, METHOD_NO_AUTH]).await?;
    Ok(SocksAuthContext {
        client_id: proxy.client_id.clone(),
        account_id: None,
    })
}

async fn read_username_password(socket: &mut TcpStream) -> anyhow::Result<(String, String)> {
    let version = socket.read_u8().await?;
    anyhow::ensure!(version == 1, "unsupported socks5 auth version {version}");
    let username_len = socket.read_u8().await? as usize;
    let mut username = vec![0_u8; username_len];
    socket.read_exact(&mut username).await?;
    let password_len = socket.read_u8().await? as usize;
    let mut password = vec![0_u8; password_len];
    socket.read_exact(&mut password).await?;
    Ok((String::from_utf8(username)?, String::from_utf8(password)?))
}

async fn handle_connect(
    state: AppState,
    auth: SocksAuthContext,
    mut socket: TcpStream,
    remote_addr: SocketAddr,
    atyp: u8,
) -> anyhow::Result<()> {
    let target = match read_socks_addr(&mut socket, atyp).await {
        Ok(addr) => addr.target(),
        Err(err) => {
            let rep = if is_addr_type_error(&err) {
                REP_ADDR_TYPE_NOT_SUPPORTED
            } else {
                REP_GENERAL_FAILURE
            };
            send_reply(&mut socket, rep, None).await?;
            return Err(err);
        }
    };
    let account_id = auth.account_id.clone();
    debug!(%remote_addr, %target, account_id = ?account_id, "socks5 connect request");
    let route = proxy_tcp::StreamRoute {
        tunnel_id: account_id
            .clone()
            .map(|id| format!("socks5:{id}"))
            .unwrap_or_else(|| "socks5".to_string()),
        client_id: auth.client_id,
    };
    let recorder = proxy_tcp::TrafficRecorder::new(&state, &route);
    let stream = match proxy_tcp::open_stream(
        state.clone(),
        &route,
        TargetProtocol::Tcp,
        target.clone(),
        remote_addr.to_string(),
    )
    .await
    {
        Ok(stream) => stream,
        Err(err) => {
            warn!(%remote_addr, %target, error = %err, "socks5 connect open stream failed");
            send_reply(&mut socket, REP_GENERAL_FAILURE, None).await?;
            return Err(err);
        }
    };
    let bind_addr = socket.local_addr().ok();
    send_reply(&mut socket, REP_SUCCEEDED, bind_addr).await?;
    debug!(%remote_addr, %target, "socks5 connect established");
    let session_guard = state.proxy_manager.register(account_id);
    let shutdown = session_guard.shutdown_rx();
    let result =
        proxy_tcp::pipe_tcp_mux_with_shutdown(socket, stream, None, Some(recorder), shutdown).await;
    drop(session_guard);
    result
}

async fn handle_udp_associate(
    state: AppState,
    mut socket: TcpStream,
    remote_addr: SocketAddr,
    udp_addr: SocketAddr,
    atyp: u8,
    auth: SocksAuthContext,
    associations: Arc<DashMap<IpAddr, Arc<SocksUdpAssociation>>>,
    sessions: Arc<DashMap<SocksUdpSessionKey, SocksUdpSession>>,
) -> anyhow::Result<()> {
    let requested = match read_socks_addr(&mut socket, atyp).await {
        Ok(addr) => addr,
        Err(err) => {
            let rep = if is_addr_type_error(&err) {
                REP_ADDR_TYPE_NOT_SUPPORTED
            } else {
                REP_GENERAL_FAILURE
            };
            send_reply(&mut socket, rep, None).await?;
            return Err(err);
        }
    };

    let association = associations
        .entry(remote_addr.ip())
        .or_insert_with(|| {
            Arc::new(SocksUdpAssociation {
                refs: AtomicUsize::new(0),
                last_seen: AtomicU64::new(now_secs()),
                client_id: auth.client_id.clone(),
                udp_tunnel_id: auth
                    .account_id
                    .clone()
                    .map(|id| format!("socks5-udp:{id}"))
                    .unwrap_or_else(|| "socks5-udp".to_string()),
            })
        })
        .clone();
    association.refs.fetch_add(1, Ordering::Relaxed);
    association.last_seen.store(now_secs(), Ordering::Relaxed);
    debug!(%remote_addr, requested = %requested.target(), udp_addr = %udp_addr, "socks5 udp associate accepted");
    send_reply(&mut socket, REP_SUCCEEDED, Some(udp_addr)).await?;

    let session_guard = state.proxy_manager.register(auth.account_id.clone());
    let mut shutdown = session_guard.shutdown_rx();
    let mut buf = [0_u8; 1024];
    loop {
        let n = tokio::select! {
            result = socket.read(&mut buf) => result?,
            _ = shutdown.changed() => break,
        };
        if n == 0 {
            break;
        }
        if let Some(association) = associations.get(&remote_addr.ip()) {
            association
                .value()
                .last_seen
                .store(now_secs(), Ordering::Relaxed);
        }
    }
    if association.refs.fetch_sub(1, Ordering::Relaxed) == 1 {
        associations.remove_if(&remote_addr.ip(), |_, current| {
            Arc::ptr_eq(current, &association)
        });
        close_sessions_for_ip(&sessions, remote_addr.ip()).await;
    }
    drop(session_guard);
    Ok(())
}

async fn run_udp_relay(
    state: AppState,
    socket: Arc<UdpSocket>,
    associations: Arc<DashMap<IpAddr, Arc<SocksUdpAssociation>>>,
    sessions: Arc<DashMap<SocksUdpSessionKey, SocksUdpSession>>,
) {
    let mut buf = vec![0_u8; 64 * 1024];

    loop {
        let (n, client_addr) = match socket.recv_from(&mut buf).await {
            Ok(value) => value,
            Err(err) => {
                warn!(error = %err, "socks5 udp recv failed");
                continue;
            }
        };

        let Some(association) = associations.get(&client_addr.ip()) else {
            debug!(%client_addr, "socks5 udp packet dropped without association");
            continue;
        };
        let route = proxy_tcp::StreamRoute {
            tunnel_id: association.value().udp_tunnel_id.clone(),
            client_id: association.value().client_id.clone(),
        };
        let recorder = proxy_tcp::TrafficRecorder::new(&state, &route);
        association
            .value()
            .last_seen
            .store(now_secs(), Ordering::Relaxed);

        let (target_addr, payload) = match parse_udp_packet(&buf[..n]) {
            Ok(packet) => packet,
            Err(err) => {
                warn!(%client_addr, error = %err, "invalid socks5 udp packet");
                continue;
            }
        };
        let target = target_addr.target();
        let key = SocksUdpSessionKey {
            client_id: route.client_id.clone(),
            client_addr,
            target: target.clone(),
        };

        let writer = if let Some(session) = sessions.get(&key) {
            session.last_seen.store(now_secs(), Ordering::Relaxed);
            session.writer.clone()
        } else {
            let stream = match proxy_tcp::open_stream(
                state.clone(),
                &route,
                TargetProtocol::Udp,
                target.clone(),
                client_addr.to_string(),
            )
            .await
            {
                Ok(stream) => stream,
                Err(err) => {
                    warn!(%client_addr, %target, error = %err, "socks5 udp open stream failed");
                    continue;
                }
            };
            debug!(%client_addr, %target, "socks5 udp stream opened");
            let (writer, mut reader) = stream.split();
            let last_seen = Arc::new(AtomicU64::new(now_secs()));
            sessions.insert(
                key.clone(),
                SocksUdpSession {
                    writer: writer.clone(),
                    last_seen: last_seen.clone(),
                },
            );

            let socket = socket.clone();
            let sessions = sessions.clone();
            let response_key = key.clone();
            let response_addr = target_addr.clone();
            let response_recorder = recorder.clone();
            tokio::spawn(async move {
                while let Some(data) = reader.recv_data().await {
                    let packet = match response_addr.encode_udp_response(&data) {
                        Ok(packet) => packet,
                        Err(err) => {
                            warn!(%client_addr, error = %err, "socks5 udp response encode failed");
                            break;
                        }
                    };
                    if let Err(err) = socket.send_to(&packet, client_addr).await {
                        warn!(%client_addr, error = %err, "socks5 udp response write failed");
                        break;
                    }
                    response_recorder.add(data.len() as u64, 0);
                    last_seen.store(now_secs(), Ordering::Relaxed);
                }
                sessions.remove(&response_key);
            });

            writer
        };

        let payload_len = payload.len() as u64;
        if let Err(err) = writer.send_data(payload).await {
            warn!(%client_addr, %target, error = %err, "socks5 udp send to mux failed");
            sessions.remove(&key);
        } else {
            recorder.add(0, payload_len);
        }
    }
}

async fn cleanup_udp_state(
    associations: Arc<DashMap<IpAddr, Arc<SocksUdpAssociation>>>,
    sessions: Arc<DashMap<SocksUdpSessionKey, SocksUdpSession>>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let now = now_secs();

        let stale_associations: Vec<_> = associations
            .iter()
            .filter_map(|entry| {
                let last_seen = entry.value().last_seen.load(Ordering::Relaxed);
                (now.saturating_sub(last_seen) > UDP_IDLE_SECS).then_some(*entry.key())
            })
            .collect();
        for key in stale_associations {
            if associations.remove(&key).is_some() {
                close_sessions_for_ip(&sessions, key).await;
            }
        }

        let stale_sessions: Vec<_> = sessions
            .iter()
            .filter_map(|entry| {
                let last_seen = entry.value().last_seen.load(Ordering::Relaxed);
                (now.saturating_sub(last_seen) > UDP_IDLE_SECS).then_some(entry.key().clone())
            })
            .collect();
        for key in stale_sessions {
            if let Some((_, session)) = sessions.remove(&key) {
                let _ = session.writer.close().await;
            }
        }
    }
}

async fn close_sessions_for_ip(
    sessions: &DashMap<SocksUdpSessionKey, SocksUdpSession>,
    client_ip: IpAddr,
) {
    let stale_sessions: Vec<_> = sessions
        .iter()
        .filter_map(|entry| {
            (entry.key().client_addr.ip() == client_ip).then_some(entry.key().clone())
        })
        .collect();
    for key in stale_sessions {
        if let Some((_, session)) = sessions.remove(&key) {
            let _ = session.writer.close().await;
        }
    }
}

async fn read_socks_addr<R>(reader: &mut R, atyp: u8) -> anyhow::Result<SocksAddr>
where
    R: AsyncRead + Unpin,
{
    let host = match atyp {
        ATYP_IPV4 => {
            let mut raw = [0_u8; 4];
            reader.read_exact(&mut raw).await?;
            Ipv4Addr::from(raw).to_string()
        }
        ATYP_IPV6 => {
            let mut raw = [0_u8; 16];
            reader.read_exact(&mut raw).await?;
            std::net::Ipv6Addr::from(raw).to_string()
        }
        ATYP_DOMAIN => {
            let len = reader.read_u8().await? as usize;
            let mut raw = vec![0_u8; len];
            reader.read_exact(&mut raw).await?;
            String::from_utf8(raw)?
        }
        _ => anyhow::bail!("unsupported socks address type {atyp}"),
    };
    let port = reader.read_u16().await?;
    Ok(SocksAddr { host, port, atyp })
}

fn parse_udp_packet(packet: &[u8]) -> anyhow::Result<(SocksAddr, Bytes)> {
    anyhow::ensure!(packet.len() >= 4, "packet too short");
    anyhow::ensure!(packet[0] == 0 && packet[1] == 0, "invalid reserved bytes");
    anyhow::ensure!(packet[2] == 0, "fragmented udp packets are not supported");

    let (addr, offset) = parse_udp_addr(packet, 3)?;
    anyhow::ensure!(packet.len() >= offset, "packet too short for payload");
    Ok((addr, Bytes::copy_from_slice(&packet[offset..])))
}

fn parse_udp_addr(packet: &[u8], offset: usize) -> anyhow::Result<(SocksAddr, usize)> {
    anyhow::ensure!(packet.len() > offset, "missing address type");
    let atyp = packet[offset];
    let mut cursor = offset + 1;
    let host = match atyp {
        ATYP_IPV4 => {
            anyhow::ensure!(packet.len() >= cursor + 4, "truncated ipv4 address");
            let raw: [u8; 4] = packet[cursor..cursor + 4].try_into()?;
            cursor += 4;
            Ipv4Addr::from(raw).to_string()
        }
        ATYP_IPV6 => {
            anyhow::ensure!(packet.len() >= cursor + 16, "truncated ipv6 address");
            let raw: [u8; 16] = packet[cursor..cursor + 16].try_into()?;
            cursor += 16;
            std::net::Ipv6Addr::from(raw).to_string()
        }
        ATYP_DOMAIN => {
            anyhow::ensure!(packet.len() > cursor, "missing domain length");
            let len = packet[cursor] as usize;
            cursor += 1;
            anyhow::ensure!(packet.len() >= cursor + len, "truncated domain");
            let host = String::from_utf8(packet[cursor..cursor + len].to_vec())?;
            cursor += len;
            host
        }
        _ => anyhow::bail!("unsupported socks address type {atyp}"),
    };
    anyhow::ensure!(packet.len() >= cursor + 2, "missing port");
    let port = u16::from_be_bytes(packet[cursor..cursor + 2].try_into()?);
    cursor += 2;
    Ok((SocksAddr { host, port, atyp }, cursor))
}

async fn send_reply(
    socket: &mut TcpStream,
    rep: u8,
    bind_addr: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let bind_addr = bind_addr.unwrap_or_else(|| SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)));
    let mut reply = vec![SOCKS_VERSION, rep, 0];
    match bind_addr.ip() {
        IpAddr::V4(ip) => {
            reply.push(ATYP_IPV4);
            reply.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            if ip.is_unspecified() {
                reply.push(ATYP_IPV4);
                reply.extend_from_slice(&Ipv4Addr::UNSPECIFIED.octets());
            } else {
                reply.push(ATYP_IPV6);
                reply.extend_from_slice(&ip.octets());
            }
        }
    }
    reply.extend_from_slice(&bind_addr.port().to_be_bytes());
    socket.write_all(&reply).await?;
    Ok(())
}

fn is_addr_type_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("unsupported socks address type")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
