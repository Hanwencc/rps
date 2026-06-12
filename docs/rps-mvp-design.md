# rps MVP 设计文档

## 1. 项目定位

`rps` 是一个使用 Rust 实现的内网穿透与代理系统，功能模型参考 `yisier/nps`，但不追求与原 Go 实现的协议兼容。MVP 目标是先落地稳定的核心链路：

外部访问者连接公网控制器 `rps-controller` 暴露的代理端口，控制器通过内网客户端 `rps-agent` 主动建立的隧道，把连接转发到客户端所在网络内的目标服务。

MVP 只保留 Docker 部署形态，不提供系统服务安装、桌面 GUI、自动更新、P2P、文件服务、secret 模式和完整 Web 管理后台。

## 2. MVP 功能范围

### 2.1 保留功能

- Docker-only 部署：
  - `rps-controller` 镜像。
  - `rps-agent` 镜像。
  - `docker-compose.yml` 示例。
- 控制器/客户端连接模型：
  - agent 主动连接 controller。
  - 使用 `client_id + psk` 认证 agent。
  - 每个 agent 至少维护一条控制连接和一条数据 mux 连接。
- 代理模式：
  - TCP 端口映射。
  - UDP 端口映射。
  - HTTP 正向代理，包含普通 HTTP 请求和 `CONNECT`。
  - SOCKS5 代理，MVP 支持 `CONNECT`。
- 基础运维能力：
  - 文件配置启动。
  - JSON/TOML/YAML 配置热重载作为后续增强，MVP 可重启生效。
  - 结构化日志。
  - 基础连接数、上下行字节计数。

### 2.2 暂不实现

- 与原 `nps/npc` 二进制互通。
- KCP。
- P2P、secret、file server。
- Web UI、多用户登录、验证码、用户注册。
- 自动申请证书、HTTPS Host 反代、多证书 SNI。
- 复杂限速、流量套餐、IP 白名单授权页面。
- 原项目自定义滑动窗口 mux 的完整复刻。

## 3. 组件架构

### 3.1 二进制

```text
rps
├── rps-controller
│   ├── bridge listener
│   ├── tunnel registry
│   ├── tcp proxy listener
│   ├── udp proxy listener
│   ├── http proxy listener
│   └── socks5 proxy listener
└── rps-agent
    ├── controller connector
    ├── control session
    ├── mux data session
    └── local target connector
```

### 3.2 Rust workspace 建议

```text
rps/
├── Cargo.toml
├── crates/
│   ├── rps-core/          # 协议、配置、模型、通用错误
│   ├── rps-mux/           # 多路复用连接
│   ├── rps-controller/    # 控制器二进制
│   └── rps-agent/         # 客户端二进制
├── configs/
│   ├── controller.toml
│   └── agent.toml
├── docker/
│   ├── Dockerfile.controller
│   └── Dockerfile.agent
└── docker-compose.yml
```

### 3.3 主要依赖建议

- `tokio`：异步运行时、TCP/UDP、任务调度。
- `serde`、`serde_json`、`toml`：配置与协议结构序列化。
- `bytes`：帧编解码缓冲。
- `tokio-util`：codec 或 framed IO。
- `tracing`、`tracing-subscriber`：日志。
- `dashmap`：在线客户端、任务和 UDP session 映射。
- `anyhow` / `thiserror`：错误处理。
- `rustls` / `tokio-rustls`：第二阶段启用 TLS bridge。

## 4. 核心数据模型

### 4.1 Client

```rust
struct ClientConfig {
    id: String,
    psk: String,
    enabled: bool,
    remark: Option<String>,
    max_connections: Option<u32>,
    compress: bool,
    encrypt: bool,
}
```

MVP 中 `encrypt` 和 `compress` 只保留配置字段，先不启用链路内二次加密和压缩。bridge 层 TLS 可作为后续独立能力。

### 4.2 Tunnel

```rust
enum TunnelMode {
    Tcp,
    Udp,
}

struct TunnelConfig {
    id: String,
    client_id: String,
    mode: TunnelMode,
    listen: ListenAddr,
    target: Option<String>,
    enabled: bool,
}

struct ProxyListenConfig {
    listen: ListenAddr,
    client_id: String,
    enabled: bool,
}

struct ServerConfig {
    bridge_addr: ListenAddr,
    http_proxy: Option<ProxyListenConfig>,
    socks5: Option<ProxyListenConfig>,
}
```

约束：

- `tcp` 和 `udp` 必须有固定 `target`。
- `http_proxy` 和 `socks5` 是 controller 级共享代理监听，各自只开一个端口，目标从请求中解析。
- `tunnels.listen` / `server.*.listen` 支持 `0.0.0.0:port`，Docker host 网络和端口映射都可用。

## 5. 配置文件

### 5.1 controller.toml 示例

```toml
[server]
bridge_addr = "0.0.0.0:8024"

[server.http_proxy]
listen = "0.0.0.0:18080"
client_id = "client-1"
enabled = true

[server.socks5]
listen = "0.0.0.0:19080"
client_id = "client-1"
enabled = true

[[clients]]
id = "client-1"
psk = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
enabled = true
remark = "demo intranet agent"
max_connections = 1024
compress = false
encrypt = false

[[tunnels]]
id = "ssh-tcp"
client_id = "client-1"
mode = "tcp"
listen = "0.0.0.0:10022"
target = "127.0.0.1:22"
enabled = true

[[tunnels]]
id = "dns-udp"
client_id = "client-1"
mode = "udp"
listen = "0.0.0.0:1053"
target = "127.0.0.1:53"
enabled = true

```

### 5.2 agent.toml 示例

```toml
[agent]
server_addr = "rps-controller:8024"
client_id = "client-1"
psk = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
reconnect_interval_secs = 5
```

## 6. Bridge 协议

MVP 使用自定义二进制帧，不兼容原 `nps` 协议。所有多字节整数使用 big-endian，便于人工排查。

### 6.1 连接阶段

agent 建立 TCP 连接到 controller：

```text
Agent -> Controller: Hello {
  magic: "RPS1",
  role: "control" | "data",
  client_id,
  psk,
  version
}

Controller -> Agent: HelloAck {
  ok: bool,
  error: Option<String>,
  server_version
}
```

MVP 至少建立两条连接：

- `control`：控制消息、心跳、断线检测。
- `data`：承载 mux streams。

后续可允许多个 `data` 连接做并发负载。

### 6.2 控制消息

控制连接消息：

```rust
enum ControlMessage {
    Ping { ts: u64 },
    Pong { ts: u64 },
    Shutdown { reason: String },
    Reload,
}
```

MVP 中 controller 不通过 control 下发任务，任务来自 controller 配置文件。agent 只负责按收到的 `OpenStream` 请求连接目标。

## 7. Mux 协议

### 7.1 帧格式

```text
0                   1                   2                   3
+--------+-------------------------------+-------------------+
| type   | stream_id u32                 | length u32        |
+--------+-------------------------------+-------------------+
| payload length bytes                                       |
+------------------------------------------------------------+
```

`type`：

- `0x01 Open`
- `0x02 OpenAck`
- `0x03 Data`
- `0x04 Close`
- `0x05 Ping`
- `0x06 Pong`
- `0x07 Error`

`stream_id`：

- controller 发起的 stream 使用奇数。
- agent 发起的 stream 使用偶数。
- MVP 实际只有 controller 为外部代理流量发起 stream。

### 7.2 Open payload

```json
{
  "tunnel_id": "ssh-tcp",
  "protocol": "tcp",
  "target": "127.0.0.1:22",
  "remote_addr": "203.0.113.10:55123",
  "timeout_ms": 5000
}
```

`protocol` 取值：

- `tcp`
- `udp`
- `http`

SOCKS5 CONNECT 和 HTTP CONNECT 最终都转为 `tcp`。

### 7.3 流关闭语义

- 任意一端读到 EOF，发送 `Close(stream_id)`。
- 收到 `Close` 后关闭本地半连接；MVP 可直接关闭整个本地 stream。
- data 连接断开时，controller 标记该 client 离线，关闭依赖它的活跃代理流。

## 8. 代理流程

### 8.1 TCP 映射

```text
外部用户 -> controller:listen
controller -> agent:mux Open { protocol=tcp, target }
agent -> 内网 target 建立 TCP
controller <-> agent <-> target 双向转发
```

验收：

- 用 TCP echo 服务验证字节透明转发。
- 多并发连接互不串流。
- agent 断线后 controller 拒绝新连接并记录日志。

### 8.2 UDP 映射

controller 为每个外部 `src_addr` 建立一个 UDP session：

```text
外部 UDP 包 -> controller UDP socket
controller 查找/创建 session
controller -> agent:mux Open { protocol=udp, target, remote_addr }
agent -> target UDP socket
双向按 datagram 转发
```

MVP 简化：

- 一个外部 `src_addr + tunnel_id` 对应一个 mux stream。
- UDP payload 在 mux `Data` 中保持单个 datagram 边界。
- session idle 超时默认 120 秒。

验收：

- 用 UDP echo 服务验证。
- 可选用 DNS 查询验证。

### 8.3 HTTP 正向代理

controller 监听 HTTP proxy 端口：

- 普通 HTTP 请求：解析请求行里的绝对 URL，目标为 `host:port`。
- `CONNECT host:port`：先返回 `HTTP/1.1 200 Connection Established`，再转 TCP 流。

MVP 不做缓存，不改写 Header，不做 Basic Auth。后续可补。

### 8.4 SOCKS5

MVP 支持：

- SOCKS5 version 5。
- `NO AUTH`。
- `CONNECT`。
- `UDP ASSOCIATE`，支持 UDP over SOCKS5 datagram 转发。
- IPv4、IPv6、domain name 地址类型。

暂不支持：

- `BIND`。
- 多账号认证。

流程：

```text
SOCKS5 握手 -> 解析目标 -> Open { protocol=tcp, target } -> 返回 succeeded -> 双向转发
SOCKS5 握手 -> UDP ASSOCIATE -> UDP datagram 解析目标 -> Open { protocol=udp, target } -> datagram 转发
```

## 9. Docker 部署

### 9.1 镜像入口

controller：

```text
/usr/local/bin/rps-controller --config /etc/rps/controller.toml
```

agent：

```text
/usr/local/bin/rps-agent --config /etc/rps/agent.toml
```

### 9.2 compose 示例目标

```yaml
services:
  rps-controller:
    image: rps-controller:latest
    network_mode: host
    volumes:
      - ./configs/controller.toml:/etc/rps/controller.toml:ro

  rps-agent:
    image: rps-agent:latest
    network_mode: host
    volumes:
      - ./configs/agent.toml:/etc/rps/agent.toml:ro
```

MVP 推荐 `network_mode: host`，因为代理服务需要暴露动态端口；后续可以提供端口映射版本示例。

## 10. 错误处理与可观测性

### 10.1 日志字段

关键日志都应包含：

- `client_id`
- `tunnel_id`
- `mode`
- `stream_id`
- `remote_addr`
- `target`
- `error`

### 10.2 计数器

MVP 内存计数即可：

- client 在线状态。
- 当前连接数。
- stream 创建总数。
- stream 错误总数。
- tunnel 入站字节数。
- tunnel 出站字节数。

后续可增加 `/metrics` Prometheus 输出。

## 11. 安全边界

- `client_id + psk` 是 MVP 的 agent 身份凭证；`client_id` 是公开标识，`psk` 是原始共享密钥。
- controller 只允许配置中存在且 enabled 的 client 登录。
- agent 不接受 controller 之外的入站连接。
- `target` 来自 controller 配置或 HTTP/SOCKS5 请求解析：
  - TCP/UDP 固定映射只允许访问配置目标。
  - HTTP proxy 和 SOCKS5 允许访问 agent 所在网络可达的任意目标，这是代理模式的预期行为。
- 后续应增加：
  - bridge TLS。
  - HTTP/SOCKS5 认证。
  - 目标 ACL。
  - IP 黑白名单。

## 12. MVP 验收标准

### 12.1 TCP

- 启动 controller 和 agent。
- controller 配置 `0.0.0.0:10022 -> agent:127.0.0.1:22` 或 echo 服务。
- 外部连接 controller `10022`，数据能到内网目标并返回。

### 12.2 UDP

- controller 配置 `0.0.0.0:1053 -> agent:127.0.0.1:53` 或 UDP echo。
- 外部发 UDP datagram，响应正确返回。
- 空闲 session 能自动清理。

### 12.3 HTTP proxy

- 配置 HTTP proxy 监听 `18080`。
- `curl -x http://controller:18080 http://target/` 成功。
- `curl -x http://controller:18080 https://target/` 通过 CONNECT 成功。

### 12.4 SOCKS5

- 配置 SOCKS5 监听 `19080`。
- `curl --socks5 controller:19080 http://target/` 成功。

## 13. 实施顺序

1. 建 Rust workspace、配置模型、日志初始化。
2. 实现 controller/agent TCP bridge 握手和 `client_id + psk` 校验。
3. 实现 mux 基础帧、stream open/data/close。
4. 实现 TCP 代理端到端。
5. 增加 Dockerfile 和 compose，完成 TCP 集成验证。
6. 实现 UDP session 代理。
7. 实现 HTTP 正向代理。
8. 实现 SOCKS5 CONNECT。
9. 补充集成测试和 README 使用说明。

## 14. 与原 nps 的对应关系

| 原 nps 模块 | rps MVP 对应 |
| --- | --- |
| `cmd/nps` | `rps-controller` |
| `cmd/npc` | `rps-agent` |
| `bridge.Bridge` | bridge listener + client registry |
| `lib/nps_mux` | `rps-mux` |
| `server/proxy/tcp.go` | TCP proxy listener |
| `server/proxy/udp.go` | UDP proxy listener + session map |
| `server/proxy/http.go` | HTTP 正向代理 |
| `server/proxy/socks5.go` | SOCKS5 CONNECT proxy |
| `conf/*.json` + `nps.conf` | `controller.toml` |
| `npc.conf` | `agent.toml` |

## 15. 第一版目录落地建议

MVP 首次编码可以先按以下最小结构创建：

```text
crates/
├── rps-core/
│   ├── src/config.rs
│   ├── src/model.rs
│   ├── src/protocol.rs
│   └── src/lib.rs
├── rps-mux/
│   ├── src/frame.rs
│   ├── src/session.rs
│   ├── src/stream.rs
│   └── src/lib.rs
├── rps-controller/
│   ├── src/bridge.rs
│   ├── src/proxy_tcp.rs
│   ├── src/proxy_udp.rs
│   ├── src/proxy_http.rs
│   ├── src/proxy_socks5.rs
│   └── src/main.rs
└── rps-agent/
    ├── src/client.rs
    ├── src/target.rs
    └── src/main.rs
```

第一条必须跑通的闭环：

```text
TCP external client
-> rps-controller tcp listener
-> rps-mux stream
-> rps-agent
-> local TCP echo server
-> response returns through same path
```
