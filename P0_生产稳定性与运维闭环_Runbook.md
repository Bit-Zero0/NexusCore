# P0 生产稳定性与运维闭环 Runbook

这份 runbook 用来收口当前阶段的 P0 项：

- 真实告警投递联调
- broker 故障恢复演练
- 启动矩阵与 CI 回归
- 生产前配置检查

正式部署与配置收口手册：

- [`生产部署与配置收口手册.md`](/home/fmy/NexusCore/生产部署与配置收口手册.md)

## 1. P0 完成标准

在准备进入下一阶段之前，至少要满足下面 4 项：

1. 告警链路已真实联调
2. 三种 broker 都做过一次本地故障恢复演练
3. 启动矩阵 smoke 与 broker contract tests 可以重复执行
4. 生产环境需要的关键配置项有明确检查清单

## 2. 真实告警投递联调

### 2.1 默认验证

```bash
./scripts/verify_alert_pipeline.sh
```

这个脚本会做几件事：

- 本地启动一个 webhook capture server
- 本地启动 `nexus-alert-relay`
- 发送一条 `warning` 和一条 `critical` 测试告警
- 验证：
  - 通用 webhook 收到至少一条
  - 飞书 webhook 收到 `warning`
  - 企业微信 webhook 收到 `critical`
  - relay 审计日志成功落盘

### 2.2 全链路 Alertmanager 模式

如果你希望把 Alertmanager 路由也一起打通：

```bash
set -a
source env/alerting.local.env
set +a

VERIFY_ALERT_MODE=alertmanager ./scripts/verify_alert_pipeline.sh
```

这个模式会额外：

- 启动 compose 里的 `alertmanager`
- 向 Alertmanager `/api/v2/alerts` 注入 `warning` 和 `critical` 测试告警
- 验证路由后的 webhook 投递结果

脚本默认使用这些地址：

- Alertmanager: `http://127.0.0.1:19093`
- Alert relay: `http://127.0.0.1:18081`
- Capture server: `http://127.0.0.1:18082`

### 2.3 成功判定

输出出现下面两类结果即可视为通过：

- `Alert delivery verified in local mode.`
- `Captured payloads:` 后能看到 `generic / feishu / wecom` 三类文件

## 3. Broker 故障恢复演练

### 3.1 先启动依赖

```bash
docker compose -f docker-compose.dev.yml up -d postgres redis rabbitmq nats
```

### 3.2 演练命令

RabbitMQ:

```bash
./scripts/run_broker_failure_drill.sh rabbitmq runtime-worker
```

NATS:

```bash
./scripts/run_broker_failure_drill.sh nats runtime-worker
```

Redis Streams:

```bash
./scripts/run_broker_failure_drill.sh redis_streams runtime-worker
```

这个脚本会：

- 用对应 broker 的 monitoring env 启动 `nexus-app`
- 验证 `/healthz`、`/metrics` 和 `runtime-worker` 的 `/api/v1/runtime/broker`
- 重启对应 broker 容器
- 等待 broker 恢复
- 再次确认应用健康检查和 broker API 正常

### 3.3 成功判定

每次演练结束后，应看到：

- `App is healthy before broker restart`
- `Broker recovery verified`

如果失败，应用日志默认在：

- `/tmp/nexus-broker-drills`

## 4. 启动矩阵与回归

### 4.1 本地回归

```bash
./scripts/smoke_runtime_matrix.sh
```

默认会覆盖：

- roles: `embedded / gateway / runtime-worker`
- brokers: `rabbitmq / nats / redis_streams`

### 4.2 CI 回归

工作流位置：

- [.github/workflows/runtime-matrix.yml](/home/fmy/NexusCore/.github/workflows/runtime-matrix.yml)

当前策略：

- broker/runtime 改动跑 `full-matrix`
- 监控、alert-relay、workflow 改动跑 `light-smoke`
- 支持 `workflow_dispatch` 手动触发

## 5. 生产前配置检查

### 5.1 Broker

必查项：

- `NEXUS_RUNTIME_BROKER_BACKEND`
- RabbitMQ:
  - `NEXUS_RUNTIME_RABBITMQ_URL`
  - `NEXUS_RUNTIME_RABBITMQ_EXCHANGE`
  - `NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX`
- NATS:
  - `NEXUS_RUNTIME_NATS_URL`
  - `NEXUS_RUNTIME_NATS_STREAM`
  - `NEXUS_RUNTIME_NATS_SUBJECT_PREFIX`
  - `NEXUS_RUNTIME_NATS_CONSUMER_PREFIX`
- Redis Streams:
  - `NEXUS_RUNTIME_REDIS_STREAMS_URL`
  - `NEXUS_RUNTIME_REDIS_STREAMS_PREFIX`
  - `NEXUS_RUNTIME_REDIS_STREAMS_GROUP_PREFIX`
  - `NEXUS_RUNTIME_REDIS_STREAMS_CONSUMER_PREFIX`
  - `NEXUS_RUNTIME_REDIS_STREAMS_PENDING_RECLAIM_IDLE_MS`

### 5.2 观测与告警

必查项：

- `/metrics` 可抓取
- Prometheus targets 为 `up`
- Grafana dashboard 已 provision
- Alertmanager `/api/v2/status` 正常
- `NEXUS_ALERT_WEBHOOK_URL`
- `NEXUS_ALERT_FEISHU_WEBHOOK_URL`
- `NEXUS_ALERT_WECOM_WEBHOOK_URL`
- `NEXUS_ALERT_AUDIT_LOG_PATH`

### 5.3 应用角色

必查项：

- `NEXUS_PROCESS_ROLE`
- `NEXUS_BIND_ADDR`
- `NEXUS_OJ_REPOSITORY`
- PostgreSQL / Redis 连接信息

## 6. 建议的 P0 验收顺序

建议按这个顺序执行：

1. `docker compose -f docker-compose.dev.yml up -d`
2. `./scripts/verify_alert_pipeline.sh`
3. `./scripts/run_broker_failure_drill.sh rabbitmq runtime-worker`
4. `./scripts/run_broker_failure_drill.sh nats runtime-worker`
5. `./scripts/run_broker_failure_drill.sh redis_streams runtime-worker`
6. `./scripts/smoke_runtime_matrix.sh`
7. 手动触发一次 `Runtime Matrix` workflow

## 7. 当前本地验证结果

基于 2026-04-07 这一轮本地验证：

- `./scripts/verify_alert_pipeline.sh` 本地模式通过
- `VERIFY_ALERT_MODE=alertmanager ./scripts/verify_alert_pipeline.sh` 全链路模式通过
- `./scripts/run_broker_failure_drill.sh nats runtime-worker` 通过
- `./scripts/run_broker_failure_drill.sh redis_streams runtime-worker` 通过
- `./scripts/run_broker_failure_drill.sh rabbitmq runtime-worker` 通过

RabbitMQ 的恢复表现是：

- broker 容器重启后，应用进程仍在
- `/healthz` 可继续访问
- `/metrics` 在重连窗口中可能短暂返回 `500`
- 随后 transport 会自动失效旧连接并重连，恢复到正常状态
