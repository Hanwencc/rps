# rps Docker 转发速度测试报告

## 测试结论

本轮在 Docker Compose 网络内测试有效转发速度，结果如下：

| 协议 | 最佳样本 | 有效吞吐 | 说明 |
| --- | --- | ---: | --- |
| TCP | 4 并发，每 worker 64 MiB | 298.39 MiB/s / 312.89 MB/s | TCP echo 往返，统计客户端收到的 echo body |
| HTTP proxy | 4 并发，每 worker 64 MiB | 1246.64 MiB/s / 1307.20 MB/s | HTTP 下行下载 `/bytes/N` |
| SOCKS5 | 4 并发，每 worker 64 MiB | 745.94 MiB/s / 782.18 MB/s | SOCKS5 CONNECT 后 HTTP 下行下载 `/bytes/N` |
| UDP | 4 并发，每 worker 0.25 MiB | 2.27 MiB/s / 2.38 MB/s | UDP datagram echo，1200 bytes/datagram |

UDP 单 session 顺序 datagram 很慢，单并发只有约 0.01 MiB/s；4 个 session 并行后达到 2.27 MiB/s。这个结果和当前 UDP 实现的 session/mux 行为强相关，说明 UDP 路径需要单独优化，不能直接类比 TCP/HTTP/SOCKS5。

## 测试口径

- 测试运行位置: `rps-loadtest` Docker 容器
- 被测路径:
  - TCP: `rps-loadtest -> rps-controller:10080 -> rps-agent -> rps-target:18081`
  - UDP: `rps-loadtest -> rps-controller:10081/udp -> rps-agent -> rps-target:18082`
  - HTTP proxy: `rps-loadtest -> rps-controller:10082 -> rps-agent -> rps-target:18083`
  - SOCKS5: `rps-loadtest -> rps-controller:10083 -> rps-agent -> rps-target:18083`
- TCP:
  - 客户端发送 payload 到 TCP echo target，同时读取 echo 回包。
  - 报告中的 bytes 是客户端成功收到的 echo bytes。
  - 实际链路承载约等于上行 payload + 下行 echo。
- HTTP proxy:
  - 请求 `GET http://rps-target:18083/bytes/N`。
  - 报告中的 bytes 是 HTTP body bytes。
- SOCKS5:
  - 先 SOCKS5 CONNECT 到 `rps-target:18083`。
  - 再请求 `GET /bytes/N`。
  - 报告中的 bytes 是 HTTP body bytes。
- UDP:
  - 每个 worker 使用一个 UDP socket。
  - 按 1200 bytes datagram 顺序 send/recv echo。
  - 报告中的 bytes 是客户端收到的 UDP echo bytes。

## 明细

### TCP

| concurrency | bytes/worker | total bytes | success | MiB/s | MB/s | elapsed |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 64 MiB | 64 MiB | 100% | 251.57 | 263.79 | 0.25s |
| 4 | 64 MiB | 256 MiB | 100% | 298.39 | 312.89 | 0.86s |

### HTTP Proxy

| concurrency | bytes/worker | total bytes | success | MiB/s | MB/s | elapsed |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 64 MiB | 64 MiB | 100% | 383.95 | 402.60 | 0.17s |
| 4 | 64 MiB | 256 MiB | 100% | 1246.64 | 1307.20 | 0.21s |

### SOCKS5

| concurrency | bytes/worker | total bytes | success | MiB/s | MB/s | elapsed |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 64 MiB | 64 MiB | 100% | 383.40 | 402.02 | 0.17s |
| 4 | 64 MiB | 256 MiB | 100% | 745.94 | 782.18 | 0.34s |

### UDP

| concurrency | bytes/worker | total bytes | datagram | success | MiB/s | MB/s | elapsed |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.25 MiB | 0.25 MiB | 1200 bytes | 100% | 0.01 | 0.01 | 17.96s |
| 4 | 0.25 MiB | 1.00 MiB | 1200 bytes | 100% | 2.27 | 2.38 | 0.44s |

## 复现命令

启动环境：

```bash
docker compose up -d --build rps-controller rps-agent rps-target
```

TCP/HTTP/SOCKS5 速度测试：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --mode throughput \
  --protocols tcp,http,socks5 \
  --levels 1,4 \
  --throughput-bytes 67108864 \
  --chunk-bytes 65536 \
  --timeout-secs 120 \
  --report /reports/rps-docker-throughput-report-main-64m.md
```

UDP 速度测试：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --mode throughput \
  --protocols udp \
  --levels 1,4 \
  --throughput-bytes 262144 \
  --udp-datagram-bytes 1200 \
  --timeout-secs 120 \
  --report /reports/rps-docker-throughput-report-udp-repeat.md
```

## 工程变更

- `rps-test-target` 新增 `GET /bytes/N`，用于生成指定大小 HTTP body。
- `rps-loadtest` 新增 `--mode throughput`。
- throughput 模式新增参数:
  - `--throughput-bytes`
  - `--chunk-bytes`
  - `--udp-datagram-bytes`

## 判断

1. TCP echo 路径的有效吞吐约 300 MiB/s，实际隧道承载还包含反向 echo 流量。
2. HTTP proxy 和 SOCKS5 下行速度较高，主要因为目标服务直接生成连续 body，测试更接近下载场景。
3. UDP 单 session 顺序 datagram 性能异常低，多 session 并行明显改善；后续应优先优化 UDP session 的 mux 转发和 datagram 批处理。
4. 当前测试是在 Docker Desktop 环境内完成，结果代表当前开发机和 Compose 网络下的观察值，不等同于 Linux 生产部署上限。
