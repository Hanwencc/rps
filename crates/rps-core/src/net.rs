//! TCP socket tuning helpers for the cross-border control/data link.
//!
//! Two problems are addressed here:
//! 1. Long-lived idle connections (e.g. the pre-warmed pool) get silently
//!    dropped by NAT gateways / stateful firewalls on the international path,
//!    surfacing later as `Connection reset by peer` (os error 104). TCP
//!    keepalive probes keep the conntrack entries alive and detect dead peers.
//! 2. High round-trip-time links need a large send/receive buffer to fill the
//!    bandwidth-delay product; otherwise a single connection cannot saturate
//!    the available bandwidth.

use std::{io, time::Duration};

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;

/// Idle time before the first keepalive probe is sent.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
/// Interval between keepalive probes once the connection is deemed idle.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Send/receive buffer size (~BDP for a 150Mbps * 200ms link).
const SOCKET_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Apply cross-border tuning: `TCP_NODELAY`, TCP keepalive and enlarged
/// send/receive buffers. Buffer sizing is best-effort — failures are ignored
/// because some platforms/containers cap the configurable size.
pub fn tune_cross_border(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;

    let sock = SockRef::from(stream);
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL);
    sock.set_tcp_keepalive(&keepalive)?;

    let _ = sock.set_send_buffer_size(SOCKET_BUFFER_BYTES);
    let _ = sock.set_recv_buffer_size(SOCKET_BUFFER_BYTES);
    Ok(())
}

/// Apply tuning for a local / same-region socket: low-latency only, no
/// keepalive or buffer changes needed.
pub fn tune_local(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)
}
