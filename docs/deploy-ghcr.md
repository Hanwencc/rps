# rps Docker 镜像发布与部署说明

本文档说明如何把 `rps-controller` 和 `rps-agent` 打包并发布到 GitHub Container Registry:

- `ghcr.io/hanwencc/rps-controller`
- `ghcr.io/hanwencc/rps-agent`

## 1. 前置条件

- 已安装 Docker。
- 已登录 GHCR。
- 当前 GitHub 账号或 token 对 `ghcr.io/hanwencc/*` 有 push 权限。

登录示例：

```bash
echo "$GHCR_TOKEN" | docker login ghcr.io -u hanwencc --password-stdin
```

`GHCR_TOKEN` 至少需要 `write:packages` 权限。

## 2. 发布镜像

在项目根目录执行：

```bash
scripts/publish-ghcr.sh v0.1.0
```

脚本会构建并推送：

```text
ghcr.io/hanwencc/rps-controller:v0.1.0
ghcr.io/hanwencc/rps-controller:latest
ghcr.io/hanwencc/rps-agent:v0.1.0
ghcr.io/hanwencc/rps-agent:latest
```

## 3. 可选参数

不推送 `latest`：

```bash
PUSH_LATEST=false scripts/publish-ghcr.sh v0.1.0
```

修改镜像命名空间：

```bash
IMAGE_NAMESPACE=ghcr.io/hanwencc scripts/publish-ghcr.sh v0.1.0
```

使用 buildx 发布多架构镜像：

```bash
RPS_PLATFORMS=linux/amd64,linux/arm64 scripts/publish-ghcr.sh v0.1.0
```

## 4. 部署示例

控制端：

```yaml
services:
  rps-controller:
    image: ghcr.io/hanwencc/rps-controller:v0.1.0
    restart: unless-stopped
    environment:
      RUST_LOG: info
    ports:
      - "8080:8080"
      - "10080:10080"
      - "10081:10081/udp"
      - "10082:10082"
      - "10083:10083"
      - "10083:10083/udp"
    volumes:
      - rps-data:/var/lib/rps

volumes:
  rps-data:
```

客户端：

```yaml
services:
  rps-agent:
    image: ghcr.io/hanwencc/rps-agent:v0.1.0
    restart: unless-stopped
    environment:
      RUST_LOG: info
      server_addr: rps-controller:8024
      client_id: client-1
      psk: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

镜像内默认配置路径：

- controller: `/etc/rps/controller.toml`
- agent: Docker 部署默认使用环境变量，不需要挂载配置文件。

如果需要覆盖配置，可以挂载本地文件：

```yaml
volumes:
  - ./configs/docker-controller.toml:/etc/rps/controller.toml:ro
```

agent 也仍然兼容配置文件方式；如果需要覆盖为文件配置，可以挂载：

```yaml
volumes:
  - ./configs/docker-agent.toml:/etc/rps/agent.toml:ro
command: ["--config", "/etc/rps/agent.toml"]
```

agent 环境变量：

- `server_addr`: controller bridge 地址，必填。
- `client_id`: client UUID，必填。
- `psk`: agent PSK，必填，建议使用 64 hex chars。
- `reconnect_interval_secs`: 重连间隔秒数，可选；不传默认 `5`。

## 5. 验证

控制端启动后检查：

```bash
curl http://127.0.0.1:8080/api/status
```

查看镜像：

```bash
docker pull ghcr.io/hanwencc/rps-controller:v0.1.0
docker pull ghcr.io/hanwencc/rps-agent:v0.1.0
```
