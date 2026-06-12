# rps Docker 转发极限测试报告

- 生成时间 Unix 秒: `1781231094`
- controller: `rps-controller`
- 并发阶梯: `1,4`
- 每 worker 请求数: `20`
- TCP/UDP payload: `64` bytes
- 单次操作超时: `10` 秒

## 结论

- `socks5udp` 本轮健康上限: 并发 `4`，吞吐 `377.46` ops/s，p95 `81.12` ms。

健康阈值定义: 成功率 >= 99%，且 p95 latency <= 1000ms。若最高阶梯仍健康，说明本轮没有打到真实极限，只能说明极限至少达到该阶梯。

## 明细

| protocol | concurrency | requests | ok | errors | success | ops/s | p50 ms | p95 ms | p99 ms | elapsed s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `socks5udp` | 1 | 20 | 20 | 0 | 100.00% | 12.19 | 82.00 | 82.04 | 82.05 | 1.64 |
| `socks5udp` | 4 | 80 | 80 | 0 | 100.00% | 377.46 | 0.37 | 81.12 | 162.78 | 0.21 |

## 复现命令

```bash
docker compose up -d --build rps-controller rps-agent rps-target
docker compose --profile loadtest run --rm rps-loadtest --levels 1,4 --requests-per-worker 20 --payload-bytes 64 --report /reports/rps-docker-limit-report.md
```

