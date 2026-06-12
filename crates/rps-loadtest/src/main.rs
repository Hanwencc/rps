use anyhow::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::{Parser, ValueEnum};
use std::{
    fmt::Write as _,
    net::{Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::timeout,
};

#[derive(Debug, Clone, Parser)]
struct Args {
    #[arg(long, value_enum, default_value = "request")]
    mode: Mode,
    #[arg(long, default_value = "rps-controller")]
    host: String,
    #[arg(long, default_value_t = 10080)]
    tcp_port: u16,
    #[arg(long, default_value_t = 10081)]
    udp_port: u16,
    #[arg(long, default_value_t = 10082)]
    http_proxy_port: u16,
    #[arg(long, default_value_t = 10083)]
    socks5_port: u16,
    #[arg(long, default_value = "rps-target")]
    target_host: String,
    #[arg(long, default_value_t = 18083)]
    target_http_port: u16,
    #[arg(long, default_value_t = 18082)]
    target_udp_port: u16,
    #[arg(long, default_value = "1,8,32,64")]
    levels: String,
    #[arg(long, default_value_t = 50)]
    requests_per_worker: usize,
    #[arg(long, default_value_t = 64)]
    payload_bytes: usize,
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    throughput_bytes: usize,
    #[arg(long, default_value_t = 64 * 1024)]
    chunk_bytes: usize,
    #[arg(long, default_value_t = 1200)]
    udp_datagram_bytes: usize,
    #[arg(long, default_value_t = 5)]
    timeout_secs: u64,
    #[arg(long)]
    http_proxy_username: Option<String>,
    #[arg(long)]
    http_proxy_password: Option<String>,
    #[arg(long)]
    socks5_username: Option<String>,
    #[arg(long)]
    socks5_password: Option<String>,
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        default_value = "tcp,udp,http,socks5"
    )]
    protocols: Vec<Protocol>,
    #[arg(long)]
    report: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Mode {
    Request,
    Throughput,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Protocol {
    Tcp,
    Udp,
    Http,
    Socks5,
    #[value(name = "socks5udp")]
    Socks5Udp,
}

impl Protocol {
    fn label(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Http => "http",
            Self::Socks5 => "socks5",
            Self::Socks5Udp => "socks5udp",
        }
    }
}

#[derive(Debug)]
struct RunResult {
    protocol: Protocol,
    concurrency: usize,
    requests: usize,
    ok: usize,
    errors: usize,
    elapsed: Duration,
    latencies: Vec<Duration>,
    error_samples: Vec<String>,
}

impl RunResult {
    fn success_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }
        (self.ok as f64 / self.requests as f64) * 100.0
    }

    fn ops_per_sec(&self) -> f64 {
        if self.elapsed.is_zero() {
            return 0.0;
        }
        self.ok as f64 / self.elapsed.as_secs_f64()
    }

    fn percentile_ms(&self, percentile: f64) -> f64 {
        if self.latencies.is_empty() {
            return 0.0;
        }
        let mut values = self.latencies.clone();
        values.sort_unstable();
        let idx = ((values.len() as f64 - 1.0) * percentile).round() as usize;
        values[idx].as_secs_f64() * 1000.0
    }

    fn is_healthy(&self) -> bool {
        self.success_rate() >= 99.0 && self.percentile_ms(0.95) <= 1000.0
    }
}

#[derive(Debug)]
struct ThroughputResult {
    protocol: Protocol,
    concurrency: usize,
    bytes_per_worker: usize,
    bytes_ok: usize,
    workers_ok: usize,
    worker_errors: usize,
    elapsed: Duration,
    error_samples: Vec<String>,
}

impl ThroughputResult {
    fn total_expected_bytes(&self) -> usize {
        self.concurrency * self.bytes_per_worker
    }

    fn success_rate(&self) -> f64 {
        if self.concurrency == 0 {
            return 0.0;
        }
        (self.workers_ok as f64 / self.concurrency as f64) * 100.0
    }

    fn mib_per_sec(&self) -> f64 {
        if self.elapsed.is_zero() {
            return 0.0;
        }
        (self.bytes_ok as f64 / 1024.0 / 1024.0) / self.elapsed.as_secs_f64()
    }

    fn mb_per_sec(&self) -> f64 {
        if self.elapsed.is_zero() {
            return 0.0;
        }
        (self.bytes_ok as f64 / 1_000_000.0) / self.elapsed.as_secs_f64()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let levels = parse_levels(&args.levels)?;
    let args = Arc::new(args);

    if args.mode == Mode::Throughput {
        let mut results = Vec::new();
        for protocol in args.protocols.iter().copied() {
            for concurrency in levels.iter().copied() {
                let result = run_throughput(args.clone(), protocol, concurrency).await;
                print_throughput_progress(&result);
                results.push(result);
            }
        }
        let report = render_throughput_report(&args, &levels, &results);
        if let Some(path) = &args.report {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(path, &report).await?;
        }
        println!("{report}");
        return Ok(());
    }

    let mut results = Vec::new();

    for protocol in args.protocols.iter().copied() {
        for concurrency in levels.iter().copied() {
            let result = run_protocol(args.clone(), protocol, concurrency).await;
            print_progress(&result);
            results.push(result);
        }
    }

    let report = render_report(&args, &levels, &results);
    if let Some(path) = &args.report {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, &report).await?;
    }
    println!("{report}");
    Ok(())
}

fn parse_levels(input: &str) -> anyhow::Result<Vec<usize>> {
    let mut levels = Vec::new();
    for value in input.split(',') {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        levels.push(
            value
                .parse()
                .with_context(|| format!("invalid level {value}"))?,
        );
    }
    anyhow::ensure!(
        !levels.is_empty(),
        "at least one concurrency level is required"
    );
    Ok(levels)
}

async fn run_protocol(args: Arc<Args>, protocol: Protocol, concurrency: usize) -> RunResult {
    let started = Instant::now();
    let mut tasks = Vec::with_capacity(concurrency);
    for worker in 0..concurrency {
        let args = args.clone();
        tasks.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(args.requests_per_worker);
            let mut ok = 0;
            let mut errors = 0;
            let mut samples = Vec::new();

            if protocol == Protocol::Socks5Udp {
                let result = run_socks5_udp_worker(args.as_ref(), worker).await;
                return match result {
                    Ok((worker_ok, worker_latencies)) => (worker_ok, 0, worker_latencies, samples),
                    Err(err) => {
                        samples.push(err.to_string());
                        (0, args.requests_per_worker, latencies, samples)
                    }
                };
            }

            for seq in 0..args.requests_per_worker {
                let op_started = Instant::now();
                let result = timeout(
                    Duration::from_secs(args.timeout_secs),
                    run_one(args.as_ref(), protocol, worker, seq),
                )
                .await
                .unwrap_or_else(|_| anyhow::bail!("operation timed out"));
                match result {
                    Ok(()) => {
                        ok += 1;
                        latencies.push(op_started.elapsed());
                    }
                    Err(err) => {
                        errors += 1;
                        if samples.len() < 5 {
                            samples.push(err.to_string());
                        }
                    }
                }
            }
            (ok, errors, latencies, samples)
        }));
    }

    let mut ok = 0;
    let mut errors = 0;
    let mut latencies = Vec::with_capacity(concurrency * args.requests_per_worker);
    let mut error_samples = Vec::new();
    for task in tasks {
        match task.await {
            Ok((task_ok, task_errors, task_latencies, task_samples)) => {
                ok += task_ok;
                errors += task_errors;
                latencies.extend(task_latencies);
                for sample in task_samples {
                    if error_samples.len() < 8 {
                        error_samples.push(sample);
                    }
                }
            }
            Err(err) => {
                errors += args.requests_per_worker;
                if error_samples.len() < 8 {
                    error_samples.push(err.to_string());
                }
            }
        }
    }

    RunResult {
        protocol,
        concurrency,
        requests: concurrency * args.requests_per_worker,
        ok,
        errors,
        elapsed: started.elapsed(),
        latencies,
        error_samples,
    }
}

async fn run_socks5_udp_worker(
    args: &Args,
    worker: usize,
) -> anyhow::Result<(usize, Vec<Duration>)> {
    let (_control, socket) = timeout(
        Duration::from_secs(args.timeout_secs),
        socks5_udp_associate(args),
    )
    .await
    .unwrap_or_else(|_| anyhow::bail!("socks5 udp associate timed out"))?;
    let mut latencies = Vec::with_capacity(args.requests_per_worker);
    let mut ok = 0;

    for seq in 0..args.requests_per_worker {
        let op_started = Instant::now();
        timeout(
            Duration::from_secs(args.timeout_secs),
            socks5_udp_echo_with_socket(args, &socket, worker, seq),
        )
        .await
        .unwrap_or_else(|_| anyhow::bail!("operation timed out"))?;
        ok += 1;
        latencies.push(op_started.elapsed());
    }

    Ok((ok, latencies))
}

async fn run_throughput(
    args: Arc<Args>,
    protocol: Protocol,
    concurrency: usize,
) -> ThroughputResult {
    let started = Instant::now();
    let mut tasks = Vec::with_capacity(concurrency);
    for worker in 0..concurrency {
        let args = args.clone();
        tasks.push(tokio::spawn(async move {
            timeout(
                Duration::from_secs(args.timeout_secs),
                run_one_throughput(args.as_ref(), protocol, worker),
            )
            .await
            .unwrap_or_else(|_| anyhow::bail!("throughput operation timed out"))
        }));
    }

    let mut bytes_ok = 0;
    let mut workers_ok = 0;
    let mut worker_errors = 0;
    let mut error_samples = Vec::new();
    for task in tasks {
        match task.await {
            Ok(Ok(bytes)) => {
                bytes_ok += bytes;
                workers_ok += 1;
            }
            Ok(Err(err)) => {
                worker_errors += 1;
                if error_samples.len() < 8 {
                    error_samples.push(err.to_string());
                }
            }
            Err(err) => {
                worker_errors += 1;
                if error_samples.len() < 8 {
                    error_samples.push(err.to_string());
                }
            }
        }
    }

    ThroughputResult {
        protocol,
        concurrency,
        bytes_per_worker: args.throughput_bytes,
        bytes_ok,
        workers_ok,
        worker_errors,
        elapsed: started.elapsed(),
        error_samples,
    }
}

async fn run_one(args: &Args, protocol: Protocol, worker: usize, seq: usize) -> anyhow::Result<()> {
    match protocol {
        Protocol::Tcp => tcp_echo(args, worker, seq).await,
        Protocol::Udp => udp_echo(args, worker, seq).await,
        Protocol::Http => http_proxy(args).await,
        Protocol::Socks5 => socks5_http(args).await,
        Protocol::Socks5Udp => socks5_udp_echo(args, worker, seq).await,
    }
}

async fn run_one_throughput(
    args: &Args,
    protocol: Protocol,
    worker: usize,
) -> anyhow::Result<usize> {
    match protocol {
        Protocol::Tcp => tcp_throughput(args, worker).await,
        Protocol::Udp => udp_throughput(args, worker).await,
        Protocol::Http => http_proxy_throughput(args).await,
        Protocol::Socks5 => socks5_throughput(args).await,
        Protocol::Socks5Udp => socks5_udp_throughput(args, worker).await,
    }
}

async fn tcp_throughput(args: &Args, worker: usize) -> anyhow::Result<usize> {
    let stream = TcpStream::connect((args.host.as_str(), args.tcp_port)).await?;
    let (mut reader, mut writer) = stream.into_split();
    let total = args.throughput_bytes;
    let chunk = payload(args.chunk_bytes, worker, 0);

    let write_task = tokio::spawn(async move {
        let mut remaining = total;
        while remaining > 0 {
            let n = remaining.min(chunk.len());
            writer.write_all(&chunk[..n]).await?;
            remaining -= n;
        }
        writer.shutdown().await?;
        anyhow::Ok(())
    });

    let read_task = tokio::spawn(async move {
        let mut received = 0;
        let mut buf = vec![0_u8; 64 * 1024];
        while received < total {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            received += n;
        }
        anyhow::ensure!(
            received == total,
            "tcp throughput received {received}/{total}"
        );
        anyhow::Ok(received)
    });

    let (_, received) = tokio::try_join!(write_task, read_task)?;
    received
}

async fn udp_throughput(args: &Args, worker: usize) -> anyhow::Result<usize> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect((args.host.as_str(), args.udp_port)).await?;
    let datagram_len = args.udp_datagram_bytes.min(args.throughput_bytes).max(1);
    let payload = payload(datagram_len, worker, 0);
    let mut received = 0;
    let mut response = vec![0_u8; datagram_len + 1024];

    while received < args.throughput_bytes {
        let n = (args.throughput_bytes - received).min(datagram_len);
        socket.send(&payload[..n]).await?;
        let got = socket.recv(&mut response).await?;
        anyhow::ensure!(got == n, "udp throughput datagram mismatch {got}/{n}");
        received += got;
    }

    Ok(received)
}

async fn http_proxy_throughput(args: &Args) -> anyhow::Result<usize> {
    let mut stream = TcpStream::connect((args.host.as_str(), args.http_proxy_port)).await?;
    let target = format!("{}:{}", args.target_host, args.target_http_port);
    let auth = http_proxy_auth_header(args)?;
    let request = format!(
        "GET http://{target}/bytes/{} HTTP/1.1\r\nHost: {target}\r\n{auth}Connection: close\r\n\r\n",
        args.throughput_bytes,
    );
    stream.write_all(request.as_bytes()).await?;
    read_http_body_len(stream, args.throughput_bytes).await
}

async fn socks5_throughput(args: &Args) -> anyhow::Result<usize> {
    let mut stream = socks5_connect(args).await?;
    let target = format!("{}:{}", args.target_host, args.target_http_port);
    let request = format!(
        "GET /bytes/{} HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n",
        args.throughput_bytes
    );
    stream.write_all(request.as_bytes()).await?;
    read_http_body_len(stream, args.throughput_bytes).await
}

async fn socks5_udp_throughput(args: &Args, worker: usize) -> anyhow::Result<usize> {
    let (_control, socket) = socks5_udp_associate(args).await?;
    let datagram_len = args.udp_datagram_bytes.min(args.throughput_bytes).max(1);
    let payload = payload(datagram_len, worker, 0);
    let mut received = 0;
    let mut response = vec![0_u8; datagram_len + 1024];

    while received < args.throughput_bytes {
        let n = (args.throughput_bytes - received).min(datagram_len);
        let packet = build_socks_udp_packet(args, &payload[..n])?;
        socket.send(&packet).await?;
        let got = socket.recv(&mut response).await?;
        let data = parse_socks_udp_payload(&response[..got])?;
        anyhow::ensure!(
            data == &payload[..n],
            "socks5 udp throughput datagram mismatch"
        );
        received += data.len();
    }

    Ok(received)
}

async fn tcp_echo(args: &Args, worker: usize, seq: usize) -> anyhow::Result<()> {
    let mut stream = TcpStream::connect((args.host.as_str(), args.tcp_port)).await?;
    let payload = payload(args.payload_bytes, worker, seq);
    stream.write_all(&payload).await?;
    let mut response = vec![0; payload.len()];
    stream.read_exact(&mut response).await?;
    anyhow::ensure!(response == payload, "tcp echo mismatch");
    Ok(())
}

async fn udp_echo(args: &Args, worker: usize, seq: usize) -> anyhow::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect((args.host.as_str(), args.udp_port)).await?;
    let payload = payload(args.payload_bytes, worker, seq);
    socket.send(&payload).await?;
    let mut response = vec![0; payload.len() + 1024];
    let n = socket.recv(&mut response).await?;
    anyhow::ensure!(&response[..n] == payload.as_slice(), "udp echo mismatch");
    Ok(())
}

async fn socks5_udp_echo(args: &Args, worker: usize, seq: usize) -> anyhow::Result<()> {
    let (_control, socket) = socks5_udp_associate(args).await?;
    socks5_udp_echo_with_socket(args, &socket, worker, seq).await
}

async fn socks5_udp_echo_with_socket(
    args: &Args,
    socket: &UdpSocket,
    worker: usize,
    seq: usize,
) -> anyhow::Result<()> {
    let payload = payload(args.payload_bytes, worker, seq);
    let packet = build_socks_udp_packet(args, &payload)?;
    socket.send(&packet).await?;
    let mut response = vec![0_u8; payload.len() + 1024];
    let n = socket.recv(&mut response).await?;
    let data = parse_socks_udp_payload(&response[..n])?;
    anyhow::ensure!(data == payload.as_slice(), "socks5 udp echo mismatch");
    Ok(())
}

async fn http_proxy(args: &Args) -> anyhow::Result<()> {
    let mut stream = TcpStream::connect((args.host.as_str(), args.http_proxy_port)).await?;
    let target = format!("{}:{}", args.target_host, args.target_http_port);
    let auth = http_proxy_auth_header(args)?;
    let request = format!(
        "GET http://{target}/ HTTP/1.1\r\nHost: {target}\r\n{auth}Connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let response = read_to_end(stream).await?;
    let text = String::from_utf8_lossy(&response);
    anyhow::ensure!(
        text.contains("200 OK") && text.ends_with("OK"),
        "bad http proxy response"
    );
    Ok(())
}

async fn socks5_http(args: &Args) -> anyhow::Result<()> {
    let mut stream = socks5_connect(args).await?;
    let target = format!("{}:{}", args.target_host, args.target_http_port);
    let request = format!("GET / HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let response = read_to_end(stream).await?;
    let text = String::from_utf8_lossy(&response);
    anyhow::ensure!(
        text.contains("200 OK") && text.ends_with("OK"),
        "bad socks5 response"
    );
    Ok(())
}

async fn socks5_connect(args: &Args) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect((args.host.as_str(), args.socks5_port)).await?;
    socks5_authenticate(args, &mut stream).await?;

    let host = args.target_host.as_bytes();
    anyhow::ensure!(host.len() <= u8::MAX as usize, "target host too long");
    let mut request = Vec::with_capacity(host.len() + 7);
    request.extend_from_slice(&[5, 1, 0, 3, host.len() as u8]);
    request.extend_from_slice(host);
    request.extend_from_slice(&args.target_http_port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    anyhow::ensure!(header[0] == 5 && header[1] == 0, "socks5 connect failed");
    let _ = read_socks_bound_addr(&mut stream, header[3]).await?;
    Ok(stream)
}

async fn socks5_udp_associate(args: &Args) -> anyhow::Result<(TcpStream, UdpSocket)> {
    let mut stream = TcpStream::connect((args.host.as_str(), args.socks5_port)).await?;
    socks5_authenticate(args, &mut stream).await?;

    stream.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).await?;

    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    anyhow::ensure!(
        header[0] == 5 && header[1] == 0,
        "socks5 udp associate failed"
    );
    let (relay_host, relay_port) = read_socks_bound_addr(&mut stream, header[3]).await?;
    let relay_host = if relay_host == "0.0.0.0" || relay_host == "::" {
        args.host.clone()
    } else {
        relay_host
    };

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect((relay_host.as_str(), relay_port)).await?;
    Ok((stream, socket))
}

fn http_proxy_auth_header(args: &Args) -> anyhow::Result<String> {
    let Some(username) = &args.http_proxy_username else {
        anyhow::ensure!(
            args.http_proxy_password.is_none(),
            "--http-proxy-password requires --http-proxy-username"
        );
        return Ok(String::new());
    };
    let password = args
        .http_proxy_password
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--http-proxy-username requires --http-proxy-password"))?;
    let encoded = BASE64.encode(format!("{username}:{password}"));
    Ok(format!("Proxy-Authorization: Basic {encoded}\r\n"))
}

async fn socks5_authenticate(args: &Args, stream: &mut TcpStream) -> anyhow::Result<()> {
    let has_auth = args.socks5_username.is_some() || args.socks5_password.is_some();
    if has_auth {
        stream.write_all(&[5, 2, 0, 2]).await?;
    } else {
        stream.write_all(&[5, 1, 0]).await?;
    }

    let mut handshake = [0; 2];
    stream.read_exact(&mut handshake).await?;
    anyhow::ensure!(handshake[0] == 5, "bad socks5 auth version");
    match handshake[1] {
        0 => Ok(()),
        2 => {
            let username = args.socks5_username.as_ref().ok_or_else(|| {
                anyhow::anyhow!("socks5 server requires username/password authentication")
            })?;
            let password = args
                .socks5_password
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--socks5-username requires --socks5-password"))?;
            anyhow::ensure!(
                username.len() <= u8::MAX as usize,
                "socks5 username too long"
            );
            anyhow::ensure!(
                password.len() <= u8::MAX as usize,
                "socks5 password too long"
            );

            let mut request = Vec::with_capacity(username.len() + password.len() + 3);
            request.push(1);
            request.push(username.len() as u8);
            request.extend_from_slice(username.as_bytes());
            request.push(password.len() as u8);
            request.extend_from_slice(password.as_bytes());
            stream.write_all(&request).await?;

            let mut response = [0; 2];
            stream.read_exact(&mut response).await?;
            anyhow::ensure!(
                response == [1, 0],
                "socks5 username/password authentication failed"
            );
            Ok(())
        }
        0xff => anyhow::bail!("socks5 auth negotiation failed"),
        method => anyhow::bail!("unsupported socks5 auth method {method}"),
    }
}

async fn read_socks_bound_addr(stream: &mut TcpStream, atyp: u8) -> anyhow::Result<(String, u16)> {
    let host = match atyp {
        1 => {
            let mut raw = [0; 4];
            stream.read_exact(&mut raw).await?;
            Ipv4Addr::from(raw).to_string()
        }
        3 => {
            let mut len = [0; 1];
            stream.read_exact(&mut len).await?;
            let mut raw = vec![0; len[0] as usize];
            stream.read_exact(&mut raw).await?;
            String::from_utf8(raw)?
        }
        4 => {
            let mut raw = [0; 16];
            stream.read_exact(&mut raw).await?;
            Ipv6Addr::from(raw).to_string()
        }
        _ => anyhow::bail!("unsupported socks5 bind address type {atyp}"),
    };
    let mut port = [0; 2];
    stream.read_exact(&mut port).await?;
    Ok((host, u16::from_be_bytes(port)))
}

fn build_socks_udp_packet(args: &Args, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    let host = args.target_host.as_bytes();
    anyhow::ensure!(host.len() <= u8::MAX as usize, "target host too long");
    let mut packet = Vec::with_capacity(payload.len() + host.len() + 7);
    packet.extend_from_slice(&[0, 0, 0, 3, host.len() as u8]);
    packet.extend_from_slice(host);
    packet.extend_from_slice(&args.target_udp_port.to_be_bytes());
    packet.extend_from_slice(payload);
    Ok(packet)
}

fn parse_socks_udp_payload(packet: &[u8]) -> anyhow::Result<&[u8]> {
    anyhow::ensure!(packet.len() >= 4, "socks5 udp packet too short");
    anyhow::ensure!(
        packet[0] == 0 && packet[1] == 0 && packet[2] == 0,
        "bad socks5 udp header"
    );
    let mut cursor = 4;
    match packet[3] {
        1 => cursor += 4,
        3 => {
            anyhow::ensure!(packet.len() > cursor, "missing socks5 udp domain length");
            let len = packet[cursor] as usize;
            cursor += 1 + len;
        }
        4 => cursor += 16,
        atyp => anyhow::bail!("unsupported socks5 udp address type {atyp}"),
    }
    anyhow::ensure!(packet.len() >= cursor + 2, "truncated socks5 udp packet");
    cursor += 2;
    Ok(&packet[cursor..])
}

async fn read_to_end(mut stream: TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    Ok(response)
}

async fn read_http_body_len(mut stream: TcpStream, expected: usize) -> anyhow::Result<usize> {
    let mut header = Vec::with_capacity(4096);
    let mut buf = vec![0_u8; 64 * 1024];
    let mut body_len = 0;
    let mut header_done = false;

    while body_len < expected {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if header_done {
            body_len += n;
            continue;
        }
        header.extend_from_slice(&buf[..n]);
        if let Some(pos) = header.windows(4).position(|w| w == b"\r\n\r\n") {
            let header_text = String::from_utf8_lossy(&header[..pos]);
            anyhow::ensure!(
                header_text.contains("200 OK"),
                "http throughput response not 200"
            );
            body_len += header.len().saturating_sub(pos + 4);
            header_done = true;
        }
        anyhow::ensure!(
            header.len() <= 64 * 1024,
            "http throughput header too large"
        );
    }

    anyhow::ensure!(
        body_len == expected,
        "http throughput body {body_len}/{expected}"
    );
    Ok(body_len)
}

fn payload(len: usize, worker: usize, seq: usize) -> Vec<u8> {
    let prefix = format!("rps-loadtest:{worker}:{seq}:");
    let mut payload = prefix.into_bytes();
    payload.resize(len.max(payload.len()), b'x');
    payload
}

fn print_progress(result: &RunResult) {
    eprintln!(
        "{} c={} ok={}/{} rate={:.2}% ops/s={:.2} p95={:.2}ms",
        result.protocol.label(),
        result.concurrency,
        result.ok,
        result.requests,
        result.success_rate(),
        result.ops_per_sec(),
        result.percentile_ms(0.95)
    );
}

fn print_throughput_progress(result: &ThroughputResult) {
    eprintln!(
        "{} c={} workers={}/{} bytes={}/{} MiB/s={:.2}",
        result.protocol.label(),
        result.concurrency,
        result.workers_ok,
        result.concurrency,
        result.bytes_ok,
        result.total_expected_bytes(),
        result.mib_per_sec()
    );
}

fn render_report(args: &Args, levels: &[usize], results: &[RunResult]) -> String {
    let mut report = String::new();
    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let _ = writeln!(report, "# rps Docker 转发极限测试报告\n");
    let _ = writeln!(report, "- 生成时间 Unix 秒: `{generated_at}`");
    let _ = writeln!(report, "- controller: `{}`", args.host);
    let _ = writeln!(report, "- 并发阶梯: `{}`", levels_to_string(levels));
    let _ = writeln!(report, "- 每 worker 请求数: `{}`", args.requests_per_worker);
    let _ = writeln!(report, "- TCP/UDP payload: `{}` bytes", args.payload_bytes);
    let _ = writeln!(report, "- 单次操作超时: `{}` 秒\n", args.timeout_secs);

    let _ = writeln!(report, "## 结论\n");
    for protocol in &args.protocols {
        let protocol_results: Vec<_> = results
            .iter()
            .filter(|result| result.protocol == *protocol)
            .collect();
        if let Some(last_healthy) = protocol_results
            .iter()
            .rev()
            .find(|result| result.is_healthy())
        {
            let _ = writeln!(
                report,
                "- `{}` 本轮健康上限: 并发 `{}`，吞吐 `{:.2}` ops/s，p95 `{:.2}` ms。",
                protocol.label(),
                last_healthy.concurrency,
                last_healthy.ops_per_sec(),
                last_healthy.percentile_ms(0.95)
            );
        } else {
            let _ = writeln!(
                report,
                "- `{}` 在本轮阶梯内没有达到健康阈值。",
                protocol.label()
            );
        }
    }
    let _ = writeln!(
        report,
        "\n健康阈值定义: 成功率 >= 99%，且 p95 latency <= 1000ms。若最高阶梯仍健康，说明本轮没有打到真实极限，只能说明极限至少达到该阶梯。\n"
    );

    let _ = writeln!(report, "## 明细\n");
    let _ = writeln!(
        report,
        "| protocol | concurrency | requests | ok | errors | success | ops/s | p50 ms | p95 ms | p99 ms | elapsed s |"
    );
    let _ = writeln!(
        report,
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for result in results {
        let _ = writeln!(
            report,
            "| `{}` | {} | {} | {} | {} | {:.2}% | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
            result.protocol.label(),
            result.concurrency,
            result.requests,
            result.ok,
            result.errors,
            result.success_rate(),
            result.ops_per_sec(),
            result.percentile_ms(0.50),
            result.percentile_ms(0.95),
            result.percentile_ms(0.99),
            result.elapsed.as_secs_f64()
        );
    }

    let failures: Vec<_> = results
        .iter()
        .filter(|result| !result.error_samples.is_empty())
        .collect();
    if !failures.is_empty() {
        let _ = writeln!(report, "\n## 错误样本\n");
        for result in failures {
            let _ = writeln!(
                report,
                "### `{}` concurrency `{}`",
                result.protocol.label(),
                result.concurrency
            );
            for sample in &result.error_samples {
                let _ = writeln!(report, "- `{}`", sample.replace('`', "'"));
            }
        }
    }

    let _ = writeln!(
        report,
        "\n## 复现命令\n\n```bash\ndocker compose up -d --build rps-controller rps-agent rps-target\ndocker compose --profile loadtest run --rm rps-loadtest --levels {} --requests-per-worker {} --payload-bytes {} --report /reports/rps-docker-limit-report.md\n```\n",
        levels_to_string(levels),
        args.requests_per_worker,
        args.payload_bytes
    );
    report
}

fn render_throughput_report(args: &Args, levels: &[usize], results: &[ThroughputResult]) -> String {
    let mut report = String::new();
    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let _ = writeln!(report, "# rps Docker 转发速度测试报告\n");
    let _ = writeln!(report, "- 生成时间 Unix 秒: `{generated_at}`");
    let _ = writeln!(report, "- controller: `{}`", args.host);
    let _ = writeln!(report, "- 并发阶梯: `{}`", levels_to_string(levels));
    let _ = writeln!(
        report,
        "- 每 worker 传输量: `{:.2}` MiB",
        args.throughput_bytes as f64 / 1024.0 / 1024.0
    );
    let _ = writeln!(report, "- TCP chunk: `{}` bytes", args.chunk_bytes);
    let _ = writeln!(
        report,
        "- UDP datagram: `{}` bytes",
        args.udp_datagram_bytes
    );
    let _ = writeln!(report, "- 单 worker 超时: `{}` 秒\n", args.timeout_secs);

    let _ = writeln!(report, "## 结论\n");
    for protocol in &args.protocols {
        let best = results
            .iter()
            .filter(|result| result.protocol == *protocol && result.worker_errors == 0)
            .max_by(|a, b| a.mib_per_sec().total_cmp(&b.mib_per_sec()));
        if let Some(best) = best {
            let _ = writeln!(
                report,
                "- `{}` 本轮最佳吞吐: 并发 `{}`，`{:.2}` MiB/s (`{:.2}` MB/s)，成功率 `{:.2}%`。",
                protocol.label(),
                best.concurrency,
                best.mib_per_sec(),
                best.mb_per_sec(),
                best.success_rate()
            );
        } else {
            let _ = writeln!(
                report,
                "- `{}` 本轮没有完全成功的速度样本。",
                protocol.label()
            );
        }
    }

    let _ = writeln!(report, "\n## 明细\n");
    let _ = writeln!(
        report,
        "| protocol | concurrency | bytes/worker MiB | total expected MiB | bytes ok MiB | workers ok | errors | success | MiB/s | MB/s | elapsed s |"
    );
    let _ = writeln!(
        report,
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for result in results {
        let _ = writeln!(
            report,
            "| `{}` | {} | {:.2} | {:.2} | {:.2} | {} | {} | {:.2}% | {:.2} | {:.2} | {:.2} |",
            result.protocol.label(),
            result.concurrency,
            result.bytes_per_worker as f64 / 1024.0 / 1024.0,
            result.total_expected_bytes() as f64 / 1024.0 / 1024.0,
            result.bytes_ok as f64 / 1024.0 / 1024.0,
            result.workers_ok,
            result.worker_errors,
            result.success_rate(),
            result.mib_per_sec(),
            result.mb_per_sec(),
            result.elapsed.as_secs_f64()
        );
    }

    let failures: Vec<_> = results
        .iter()
        .filter(|result| !result.error_samples.is_empty())
        .collect();
    if !failures.is_empty() {
        let _ = writeln!(report, "\n## 错误样本\n");
        for result in failures {
            let _ = writeln!(
                report,
                "### `{}` concurrency `{}`",
                result.protocol.label(),
                result.concurrency
            );
            for sample in &result.error_samples {
                let _ = writeln!(report, "- `{}`", sample.replace('`', "'"));
            }
        }
    }

    let _ = writeln!(
        report,
        "\n## 复现命令\n\n```bash\ndocker compose up -d --build rps-controller rps-agent rps-target\ndocker compose --profile loadtest run --rm rps-loadtest --mode throughput --levels {} --throughput-bytes {} --chunk-bytes {} --udp-datagram-bytes {} --timeout-secs {} --report /reports/rps-docker-throughput-report.md\n```\n",
        levels_to_string(levels),
        args.throughput_bytes,
        args.chunk_bytes,
        args.udp_datagram_bytes,
        args.timeout_secs
    );
    report
}

fn levels_to_string(levels: &[usize]) -> String {
    levels
        .iter()
        .map(|level| level.to_string())
        .collect::<Vec<_>>()
        .join(",")
}
