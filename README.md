# rps

`rps` 是一个用 Rust 实现的反向代理/内网穿透 MVP，设计上参考 `yisier/nps` 的连接模型：

```text
外部访问者 -> controller 代理端口 -> agent-controller 加密隧道 -> agent -> 目标服务
```

当前功能：

- controller/agent 双端架构。
- agent 主动连接 controller bridge。
- agent-controller 通道使用 Noise + PSK 加密握手。
- TCP 隧道。
- UDP 隧道。
- HTTP 正向代理。
- SOCKS5 代理，包含 CONNECT 和 UDP ASSOCIATE。
- SQLite 保存 client、隧道、代理账号、流量统计、在线状态快照。
- Vue3 + Tailwind 控制台。
- 控制台账号密码登录，支持 TOTP 2FA 流程。
- Docker 部署。

## 目录结构

```text
crates/
  rps-controller   controller 服务端
  rps-agent        agent 客户端
  rps-core         公共配置、协议、Noise 加密
  rps-mux          隧道多路复用
  rps-test-target  Docker 测试目标服务
  rps-loadtest     Docker 压测工具
web/               Vue3 + Tailwind 控制台
configs/           示例配置
docker/            Dockerfile
docs/              设计、部署、测试报告
scripts/           发布脚本
```

## 前置依赖

Docker 部署只需要：

- Docker
- Docker Compose

本地开发编译需要：

- Rust 工具链，需支持 edition 2024
- Node.js 和 npm

## 快速启动

在项目根目录执行：

```bash
docker compose up -d --build rps-controller rps-agent
```

启动后访问控制台：

```text
http://127.0.0.1:8080
```

默认账号：

```text
username: admin
password: change-me
```

默认开放端口：

```text
8080/tcp   Web 控制台
10080/tcp  TCP 示例隧道
10081/udp  UDP 示例隧道
10082/tcp  HTTP 代理
10083/tcp  SOCKS5 代理
10083/udp  SOCKS5 UDP relay
```

查看运行状态：

```bash
docker compose ps
docker compose logs -f rps-controller
docker compose logs -f rps-agent
```

停止服务：

```bash
docker compose down
```

如果需要清空 SQLite 数据：

```bash
docker compose down -v
```

## Docker 配置

`docker-compose.yml` 默认启动三个核心服务：

- `rps-controller`
- `rps-agent`
- `rps-target`，用于本地验证 TCP/UDP/HTTP 转发

controller 默认配置在镜像内：

```text
/etc/rps/controller.toml
```

SQLite 默认保存到：

```text
/var/lib/rps/rps.db
```

Compose 使用命名卷持久化：

```yaml
volumes:
  rps-data:
```

agent Docker 部署通过环境变量传参：

```yaml
environment:
  server_addr: rps-controller:8024
  client_id: client-1
  psk: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

字段说明：

- `server_addr`: controller bridge 地址。
- `client_id`: controller 数据库中的 client ID。
- `psk`: Noise PSK，必须是 64 位 hex 字符串，需要和 controller 中对应 client 的 psk 一致。
- `reconnect_interval_secs`: 可选，默认 `5` 秒。

公网部署时必须修改默认账号密码和默认 PSK。

## 本地编译

本地编译主要用于开发调试。正式部署优先使用 Docker。

编译 Rust：

```bash
cargo build --workspace
```

编译 release：

```bash
cargo build --release --workspace
```

单独编译 controller 和 agent：

```bash
cargo build --release -p rps-controller
cargo build --release -p rps-agent
```

前端安装依赖并构建：

```bash
cd web
npm install
npm run build
```

完整检查：

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cd web
npm run build
```

## 本地运行开发版

先构建前端：

```bash
cd web
npm run build
```

再从项目根目录启动 controller：

```bash
cargo run -p rps-controller -- --config configs/controller.toml
```

启动 agent：

```bash
cargo run -p rps-agent -- --config configs/docker-agent.toml
```

也可以用环境变量启动 agent：

```bash
server_addr=127.0.0.1:8024 \
client_id=client-1 \
psk=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
cargo run -p rps-agent
```

Windows PowerShell 示例：

```powershell
$env:server_addr = "127.0.0.1:8024"
$env:client_id = "client-1"
$env:psk = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
cargo run -p rps-agent
```

## 基本验证

未登录访问 API 应返回 `401`：

```bash
curl -i http://127.0.0.1:8080/api/status
```

登录后访问控制台：

```text
http://127.0.0.1:8080
```

Docker 环境下可以运行内置压测工具：

```bash
docker compose --profile loadtest run --rm rps-loadtest
```

吞吐测试：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --mode throughput \
  --protocols tcp,udp,http,socks5,socks5udp \
  --levels 1,4 \
  --throughput-bytes 8388608 \
  --chunk-bytes 65536 \
  --udp-datagram-bytes 1200 \
  --timeout-secs 20
```

测试报告会写入 `docs/`。

## 发布 GHCR 镜像

登录 GHCR：

```bash
echo "$GHCR_TOKEN" | docker login ghcr.io -u hanwencc --password-stdin
```

发布 controller 和 agent：

```bash
scripts/publish-ghcr.sh v0.1.0
```

默认推送：

```text
ghcr.io/hanwencc/rps-controller:v0.1.0
ghcr.io/hanwencc/rps-controller:latest
ghcr.io/hanwencc/rps-agent:v0.1.0
ghcr.io/hanwencc/rps-agent:latest
```

更多发布说明见：

```text
docs/deploy-ghcr.md
```

## 安全说明

- agent-controller bridge 使用 Noise + PSK，加密握手后再进行业务认证。
- `client_id` 会在 Noise prelude 中明文出现，用于 controller 查找对应 PSK。
- `psk` 不会在 bridge 中明文发送。
- controller SQLite 当前保存原始 `psk`，部署时应限制数据库文件权限和备份访问权限。
- 默认控制台账号密码仅用于本地示例，公网部署必须修改。
- TOTP 2FA 已支持；截图中那类安全钥匙/Passkey/WebAuthn 还没有完整实现，需要 HTTPS、安全上下文、固定 RP ID 和凭据表。

## 常用问题

### Docker build 很慢

Rust 依赖和 npm 依赖首次下载较慢，尤其在网络不稳定时可能卡很久。Docker Desktop 显示 build 成功后，可以只重启容器：

```bash
docker compose up -d --no-build rps-controller rps-agent
```

### 修改前端后 Docker 没更新

controller 镜像会把 `web/dist` 打进 `/usr/share/rps/web`。修改前端后需要重新构建 controller 镜像：

```bash
cd web
npm run build
cd ..
docker compose build rps-controller
docker compose up -d --no-build rps-controller
```

### 如何修改控制台账号密码

修改 `configs/docker-controller.toml`：

```toml
[server.web_auth]
enabled = true
username = "admin"
password = "change-me"
session_ttl_secs = 86400
```

重新构建并启动 controller：

```bash
docker compose build rps-controller
docker compose up -d --no-build rps-controller
```

