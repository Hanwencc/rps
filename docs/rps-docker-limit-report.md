# rps Docker 转发极限测试报告

## 测试结论

本轮测试环境下，按“成功率 >= 99% 且 p95 <= 1000ms”为健康阈值，观察到的健康边界如下：

| 协议 | 健康上限 | 下一阶梯表现 | 结论 |
| --- | ---: | --- | --- |
| TCP | 256 并发 | 512 并发成功率 32.52% | 健康边界在 256 到 512 之间 |
| UDP | 24 并发 | 32 并发 p95 1312.09ms | 健康边界在 24 到 32 之间 |
| HTTP proxy | 192 并发 | 256 并发成功率 72.58% | 健康边界在 192 到 256 之间 |
| SOCKS5 | 192 并发 | 256 并发成功率 95.35% | 健康边界在 192 到 256 之间 |

UDP 是当前最明显的瓶颈，吞吐长期贴近 25 ops/s，主要受当前 UDP session/mux 转发实现影响。TCP/HTTP/SOCKS5 在中高并发下能保持较高吞吐，但到 256/512 附近会出现超时，说明当前单 data mux、每请求新 stream、无背压/连接池/多 data 连接的设计已经触顶。

## 测试环境

- 部署方式: Docker Compose
- 被测服务:
  - `rps-controller`
  - `rps-agent`
  - `rps-target`
  - `rps-loadtest`
- 测试路径:
  - TCP: `rps-loadtest -> rps-controller:10080 -> rps-agent -> rps-target:18081`
  - UDP: `rps-loadtest -> rps-controller:10081/udp -> rps-agent -> rps-target:18082`
  - HTTP proxy: `rps-loadtest -> rps-controller:10082 -> rps-agent -> rps-target:18083`
  - SOCKS5: `rps-loadtest -> rps-controller:10083 -> rps-agent -> rps-target:18083`
- payload: TCP/UDP 使用 64 bytes echo payload
- HTTP/SOCKS5: 每次请求新建连接，请求 `rps-target:18083/`
- 单次操作超时: 3 到 5 秒，按具体阶梯设置

## 压测工具

新增 crate:

```text
crates/rps-loadtest
```

新增 Dockerfile:

```text
docker/Dockerfile.loadtest
```

新增 Compose profile:

```text
rps-loadtest
```

压测工具会输出：

- 请求总数
- 成功数
- 错误数
- 成功率
- ops/s
- p50/p95/p99 latency
- 错误样本
- Markdown 报告

## 关键明细

### TCP

| concurrency | requests | ok | errors | success | ops/s | p95 ms | p99 ms | 判定 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 64 | 3200 | 3200 | 0 | 100.00% | 8164.43 | 8.23 | 15.15 | healthy |
| 128 | 2560 | 2560 | 0 | 100.00% | 7594.24 | 22.18 | 26.04 | healthy |
| 256 | 5120 | 5120 | 0 | 100.00% | 3435.32 | 61.35 | 1042.59 | healthy |
| 512 | 5120 | 1665 | 3455 | 32.52% | 55.42 | 96.63 | 108.39 | failed |

TCP 在 256 并发仍满足健康阈值，但 p99 已超过 1s。512 并发出现大量超时，当前健康上限按 256 记录。

### UDP

| concurrency | requests | ok | errors | success | ops/s | p95 ms | p99 ms | 判定 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 8 | 400 | 400 | 0 | 100.00% | 24.45 | 328.98 | 329.17 | healthy |
| 16 | 480 | 480 | 0 | 100.00% | 24.74 | 657.01 | 696.35 | healthy |
| 24 | 720 | 720 | 0 | 100.00% | 25.41 | 984.09 | 985.10 | healthy |
| 32 | 1600 | 1600 | 0 | 100.00% | 25.49 | 1312.09 | 1313.05 | slow |
| 64 | 3200 | 3200 | 0 | 100.00% | 27.14 | 2541.99 | 2582.39 | slow |

UDP 没有丢包，但延迟随并发线性抬升。按 p95 <= 1000ms 阈值，健康上限是 24 并发。

### HTTP Proxy

| concurrency | requests | ok | errors | success | ops/s | p95 ms | p99 ms | 判定 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 64 | 3200 | 3200 | 0 | 100.00% | 8700.91 | 7.86 | 9.52 | healthy |
| 128 | 1280 | 1280 | 0 | 100.00% | 3145.85 | 25.67 | 43.13 | healthy |
| 192 | 1920 | 1920 | 0 | 100.00% | 8271.51 | 25.16 | 30.10 | healthy |
| 256 | 2560 | 1858 | 702 | 72.58% | 61.87 | 89.77 | 104.97 | failed |
| 512 | 5120 | 0 | 5120 | 0.00% | 0.00 | 0.00 | 0.00 | failed |

HTTP proxy 在 192 并发仍健康，256 并发开始大量超时。

### SOCKS5

| concurrency | requests | ok | errors | success | ops/s | p95 ms | p99 ms | 判定 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 64 | 3200 | 3200 | 0 | 100.00% | 5507.39 | 10.46 | 12.57 | healthy |
| 128 | 1280 | 1280 | 0 | 100.00% | 7956.19 | 44.13 | 50.83 | healthy |
| 192 | 1920 | 1920 | 0 | 100.00% | 1610.18 | 29.99 | 1039.81 | healthy |
| 256 | 2560 | 2441 | 119 | 95.35% | 240.62 | 31.79 | 1056.46 | failed |

SOCKS5 在 192 并发成功率仍为 100%，但 p99 已超过 1s。256 并发成功率跌破 99%，记录为不健康。

## 复现命令

启动被测环境：

```bash
docker compose up -d --build rps-controller rps-agent rps-target
```

基础四协议阶梯：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --levels 1,8,32,64 \
  --requests-per-worker 50 \
  --payload-bytes 64 \
  --timeout-secs 5 \
  --report /reports/rps-docker-limit-report.md
```

TCP 边界：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --protocols tcp \
  --levels 128,256,512 \
  --requests-per-worker 10 \
  --payload-bytes 64 \
  --timeout-secs 3 \
  --report /reports/rps-docker-limit-report-tcp.md
```

UDP 边界：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --protocols udp \
  --levels 12,16,24,32 \
  --requests-per-worker 30 \
  --payload-bytes 64 \
  --timeout-secs 5 \
  --report /reports/rps-docker-limit-report-udp.md
```

HTTP proxy 边界：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --protocols http \
  --levels 96,128,192,256 \
  --requests-per-worker 10 \
  --payload-bytes 64 \
  --timeout-secs 3 \
  --report /reports/rps-docker-limit-report-http.md
```

SOCKS5 边界：

```bash
docker compose --profile loadtest run --rm rps-loadtest \
  --protocols socks5 \
  --levels 128,192,256 \
  --requests-per-worker 10 \
  --payload-bytes 64 \
  --timeout-secs 3 \
  --report /reports/rps-docker-limit-report-socks5.md
```

## 当前瓶颈判断

1. UDP 当前实现是最弱路径，吞吐稳定在约 25 ops/s，延迟随并发增加接近线性上升。
2. TCP/HTTP/SOCKS5 在 192 到 256 以后开始出现明显超时，说明单 data mux 和 stream 调度在高并发下缺少足够的背压和并行 data 连接。
3. HTTP/SOCKS5 每次请求新建连接，测试结果更偏向连接建立与 stream open 压力，不代表长连接复用场景。
4. 这些数值是当前 Docker Desktop/本机环境下的观察值，不是协议理论极限；换 Linux 原生 Docker、调整 CPU/内存、增加 data mux 连接后会变化。

## 后续优化建议

- agent 和 controller 支持多个 data mux 连接，并按 stream 做负载分配。
- `open_stream` 增加队列长度、超时、拒绝原因和指标。
- UDP 使用 datagram 批处理或专用 UDP relay worker，避免所有 session 竞争同一 mux 路径。
- 控制台增加压测页面，展示 active streams、open failures、per-tunnel latency。
- 加入 Prometheus 指标，压测时同时记录 CPU、内存、连接数、stream 数和错误原因。
