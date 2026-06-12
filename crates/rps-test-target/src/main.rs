use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    tokio::spawn(tcp_echo());
    tokio::spawn(udp_echo());
    tokio::spawn(http_ok());
    std::future::pending::<()>().await;
    Ok(())
}

async fn tcp_echo() {
    if let Err(err) = tcp_echo_inner().await {
        error!(error = %err, "tcp echo stopped");
    }
}

async fn tcp_echo_inner() -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:18081").await?;
    info!("tcp echo listening on 0.0.0.0:18081");
    loop {
        let (mut socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = vec![0_u8; 8192];
            loop {
                let n = match socket.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if socket.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        });
    }
}

async fn udp_echo() {
    if let Err(err) = udp_echo_inner().await {
        error!(error = %err, "udp echo stopped");
    }
}

async fn udp_echo_inner() -> anyhow::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:18082").await?;
    info!("udp echo listening on 0.0.0.0:18082");
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let (n, remote) = socket.recv_from(&mut buf).await?;
        socket.send_to(&buf[..n], remote).await?;
    }
}

async fn http_ok() {
    if let Err(err) = http_ok_inner().await {
        error!(error = %err, "http target stopped");
    }
}

async fn http_ok_inner() -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:18083").await?;
    info!("http target listening on 0.0.0.0:18083");
    loop {
        let (mut socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = vec![0_u8; 8192];
            let n = socket.read(&mut buf).await.unwrap_or_default();
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            if let Some(bytes) = path
                .strip_prefix("/bytes/")
                .and_then(|value| value.parse::<usize>().ok())
            {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {bytes}\r\nConnection: close\r\n\r\n"
                );
                if socket.write_all(header.as_bytes()).await.is_err() {
                    return;
                }
                let chunk = vec![b'a'; 64 * 1024];
                let mut remaining = bytes;
                while remaining > 0 {
                    let n = remaining.min(chunk.len());
                    if socket.write_all(&chunk[..n]).await.is_err() {
                        return;
                    }
                    remaining -= n;
                }
            } else {
                let response =
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                let _ = socket.write_all(response).await;
            }
        });
    }
}
