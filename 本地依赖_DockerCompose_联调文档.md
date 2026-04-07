# 本地依赖 Docker Compose 联调文档

## 1. 目标

通过一个 compose 文件同时拉起本地开发常用依赖：

- PostgreSQL
- Redis
- RabbitMQ
- NATS JetStream
- Prometheus
- Alertmanager
- Grafana

compose 文件位置：

[`docker-compose.dev.yml`](/home/fmy/NexusCore/docker-compose.dev.yml)

推荐配套环境变量样板：

- RabbitMQ: [`dev.compose.rabbitmq.env`](/home/fmy/NexusCore/env/dev.compose.rabbitmq.env)
- NATS: [`dev.compose.nats.env`](/home/fmy/NexusCore/env/dev.compose.nats.env)
- Redis Streams: [`dev.compose.redis_streams.env`](/home/fmy/NexusCore/env/dev.compose.redis_streams.env)
- RabbitMQ + Monitoring: [`dev.compose.monitoring.rabbitmq.env`](/home/fmy/NexusCore/env/dev.compose.monitoring.rabbitmq.env)
- NATS + Monitoring: [`dev.compose.monitoring.nats.env`](/home/fmy/NexusCore/env/dev.compose.monitoring.nats.env)
- Redis Streams + Monitoring: [`dev.compose.monitoring.redis_streams.env`](/home/fmy/NexusCore/env/dev.compose.monitoring.redis_streams.env)

生产配置样板：

- 基础层: [`prod.base.env.example`](/home/fmy/NexusCore/env/prod.base.env.example)
- RabbitMQ: [`prod.broker.rabbitmq.env.example`](/home/fmy/NexusCore/env/prod.broker.rabbitmq.env.example)
- NATS: [`prod.broker.nats.env.example`](/home/fmy/NexusCore/env/prod.broker.nats.env.example)
- Redis Streams: [`prod.broker.redis_streams.env.example`](/home/fmy/NexusCore/env/prod.broker.redis_streams.env.example)
- 告警层: [`prod.alerting.env.example`](/home/fmy/NexusCore/env/prod.alerting.env.example)

## 2. 启动

```bash
docker compose -f docker-compose.dev.yml up -d
```

NATS 服务会以 JetStream 模式启动，并把持久化数据写到挂载卷对应的 `/data`。

停止并清理容器：

```bash
docker compose -f docker-compose.dev.yml down
```

如果你希望连卷一起清掉：

```bash
docker compose -f docker-compose.dev.yml down -v
```

## 3. 默认端口

- PostgreSQL: `127.0.0.1:55432`
- Redis: `127.0.0.1:6379`
- RabbitMQ AMQP: `127.0.0.1:5672`
- RabbitMQ 管理后台: `http://127.0.0.1:15672`
- RabbitMQ Prometheus: `http://127.0.0.1:15692/metrics`
- NATS: `127.0.0.1:4222`
- NATS 监控: `http://127.0.0.1:8222`
- NATS Exporter: 通过 compose 内部地址 `http://nats-exporter:7777/metrics` 提供给 Prometheus
- Prometheus: `http://127.0.0.1:9090`
- Alertmanager: `http://127.0.0.1:9093`
- Alert Relay: `http://127.0.0.1:18081/healthz`
- Alert Relay Status: `http://127.0.0.1:18081/status`
- Grafana: `http://127.0.0.1:3000`

## 4. 默认账号

- PostgreSQL
  - database: `nexus_code`
  - username: `postgres`
  - password: `postgres`
- RabbitMQ
  - username: `guest`
  - password: `guest`
- Grafana
  - username: `admin`
  - password: `admin`

## 5. 常用环境变量

如果使用 PostgreSQL + Redis + RabbitMQ：

```bash
export NEXUS_OJ_REPOSITORY=postgres
export NEXUS_PG_HOST=127.0.0.1
export NEXUS_PG_PORT=55432
export NEXUS_PG_DATABASE=nexus_code
export NEXUS_PG_USERNAME=postgres
export NEXUS_PG_PASSWORD=postgres
export NEXUS_REDIS_URL="redis://127.0.0.1:6379/"
export NEXUS_RUNTIME_BROKER_BACKEND=rabbitmq
export NEXUS_RUNTIME_RABBITMQ_URL="amqp://guest:guest@127.0.0.1:5672/%2f"
export NEXUS_RUNTIME_RABBITMQ_EXCHANGE="nexus.runtime"
export NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX="nexus.runtime"
```

也可以直接加载：

```bash
source env/dev.compose.rabbitmq.env
```

如果切换到 NATS：

```bash
export NEXUS_RUNTIME_BROKER_BACKEND=nats
export NEXUS_RUNTIME_NATS_URL="nats://127.0.0.1:4222"
export NEXUS_RUNTIME_NATS_STREAM="NEXUS_RUNTIME"
export NEXUS_RUNTIME_NATS_SUBJECT_PREFIX="nexus.runtime"
export NEXUS_RUNTIME_NATS_CONSUMER_PREFIX="nexus-runtime"
```

也可以直接加载：

```bash
source env/dev.compose.nats.env
```

如果切换到 Redis Streams：

```bash
export NEXUS_RUNTIME_BROKER_BACKEND=redis_streams
export NEXUS_RUNTIME_REDIS_STREAMS_URL="redis://127.0.0.1:6379/"
export NEXUS_RUNTIME_REDIS_STREAMS_PREFIX="nexus.runtime"
export NEXUS_RUNTIME_REDIS_STREAMS_GROUP_PREFIX="nexus-runtime"
export NEXUS_RUNTIME_REDIS_STREAMS_CONSUMER_PREFIX="nexus-runtime"
export NEXUS_RUNTIME_REDIS_STREAMS_PENDING_RECLAIM_IDLE_MS=1000
```

也可以直接加载：

```bash
source env/dev.compose.redis_streams.env
```

如果你希望 Prometheus 直接抓宿主机上的 `nexus-app` 指标，建议直接使用 monitoring 样板，它会把应用绑定地址切到 `0.0.0.0:8080`：

```bash
source env/dev.compose.monitoring.rabbitmq.env
```

或者：

```bash
source env/dev.compose.monitoring.nats.env
```

或者：

```bash
source env/dev.compose.monitoring.redis_streams.env
```

之后启动应用：

```bash
cargo run -p nexus-app
```

Grafana 会通过 provisioning 自动接入 Prometheus，并自动加载这份 dashboard：

[`nexus-runtime-broker-observability.json`](/home/fmy/NexusCore/monitoring/grafana/dashboards/nexus-runtime-broker-observability.json)

其中 Prometheus 默认会抓三类指标来源：

- 宿主机上的 `nexus-app`：`http://host.docker.internal:8080/metrics`
- RabbitMQ 原生 exporter：`http://rabbitmq:15692/metrics`
- NATS Prometheus exporter：`http://nats-exporter:7777/metrics`

Prometheus 还会自动加载规则文件：

- [`nexus-runtime-alerts.yml`](/home/fmy/NexusCore/monitoring/prometheus/rules/nexus-runtime-alerts.yml)

Alertmanager 使用本地配置：

- [`alertmanager.yml`](/home/fmy/NexusCore/monitoring/alertmanager/alertmanager.yml)

Alert relay 会把 Alertmanager 事件转发到以下可选通道：

- 通用 webhook
- 飞书机器人 webhook
- 企业微信机器人 webhook

默认路由策略：

- 所有告警都会转发到通用 webhook
- `severity=warning` 的告警会额外转发到飞书
- `severity=critical` 的告警会额外转发到企业微信
- 所有告警都会额外进入本地 dry-run/logger receiver

配置样板：

- [`alerting.local.env`](/home/fmy/NexusCore/env/alerting.local.env)

本地加载方式：

```bash
set -a
source env/alerting.local.env
set +a
docker compose -f docker-compose.dev.yml up -d
```

审计日志默认会写到：

- `/tmp/nexus-alert-relay/audit.jsonl`

如果当前文件超过 `NEXUS_ALERT_AUDIT_LOG_MAX_BYTES`，relay 会自动切到带时间戳的新文件。

如果 rotated 文件超过 `NEXUS_ALERT_AUDIT_LOG_MAX_FILES`，relay 会自动清理更旧的 rotated 文件。

如果 rotated 文件的修改时间超过 `NEXUS_ALERT_AUDIT_LOG_RETENTION_DAYS`，relay 也会自动清理过期文件。

排障时可以直接用脚本查询和导出审计日志：

```bash
python3 scripts/query_alert_audit.py --path /tmp/nexus-alert-relay/audit.jsonl --severity warning
python3 scripts/query_alert_audit.py --path /tmp/nexus-alert-relay --channel dry-run --format json
python3 scripts/query_alert_audit.py --alertname NexusBrokerDeadLettersDetected --export /tmp/dead-letters.jsonl
python3 scripts/query_alert_audit.py --since 15m --contains reclaim
python3 scripts/query_alert_audit.py --since 2026-04-07T07:00:00+08:00 --contains rabbitmq
python3 scripts/query_alert_audit.py --since 1d --summary alertname
python3 scripts/query_alert_audit.py --since 1d --summary severity --format json
```

本地 dry-run 调试入口：

```bash
curl -X POST http://127.0.0.1:18081/dry-run \
  -H 'Content-Type: application/json' \
  --data '{"status":"firing","alerts":[{"labels":{"alertname":"Demo","severity":"warning"}}]}'
```

Dashboard 里额外提供了一组 15 分钟安全事件汇总面板，用来快速查看：

- retry
- dead-letter
- replay
- reclaim

当前默认规则覆盖：

- `NexusBrokerOperationFailuresHigh`
- `NexusBrokerDeadLettersDetected`
- `NexusBrokerReclaimSpike`
- `NexusBrokerQueueBacklogHigh`

runtime 管理接口里现在也可以查看 replay 历史：

- `GET /api/v1/runtime/queues/replays`

## 7. 运维脚本

P0 runbook：

- [`P0_生产稳定性与运维闭环_Runbook.md`](/home/fmy/NexusCore/P0_生产稳定性与运维闭环_Runbook.md)
- [`生产部署与配置收口手册.md`](/home/fmy/NexusCore/生产部署与配置收口手册.md)

快速启动某个 broker + role：

```bash
./scripts/run_nexus_role.sh rabbitmq embedded
./scripts/run_nexus_role.sh nats runtime-worker monitoring
./scripts/run_nexus_role.sh redis_streams gateway
```

本地单独启动 Rust 版 alert relay：

```bash
cargo run -p nexus-alert-relay
```

生产前检查当前环境变量：

```bash
./scripts/check_deploy_config.sh
```

运行完整启动矩阵 smoke 回归：

```bash
./scripts/smoke_runtime_matrix.sh
```

运行真实告警投递联调：

```bash
./scripts/verify_alert_pipeline.sh
```

如果你希望把 Alertmanager 路由也一起联调：

```bash
VERIFY_ALERT_MODE=alertmanager ./scripts/verify_alert_pipeline.sh
```

运行 broker 故障恢复演练：

```bash
./scripts/run_broker_failure_drill.sh rabbitmq runtime-worker
./scripts/run_broker_failure_drill.sh nats runtime-worker
./scripts/run_broker_failure_drill.sh redis_streams runtime-worker
```

这个脚本会串行验证：

- `embedded`
- `gateway`
- `runtime-worker`

和三个 broker 的 9 组组合：

- `rabbitmq`
- `nats`
- `redis_streams`

默认会使用 `OJ_REPOSITORY_OVERRIDE=memory` 做轻量 smoke，避免把运维回归和业务数据准备强耦合。

CI 会按改动范围分层执行：

- broker/runtime 相关改动跑 full matrix
- 监控、alert-relay、workflow 相关改动跑 light smoke

`light-smoke` 里还会额外启动一次 `nexus-alert-relay` 并校验：

- `/healthz`
- `/dry-run`
- 本地 JSON 审计文件落盘

运行中的 runtime 管理视图可通过以下接口查看：

- `GET /api/v1/runtime/management/broker`

这个接口支持：

- `queue / lane / group / limit / offset` 过滤和分页
- `task_id / delivery_id` 检索 dead-letter 和 replay 历史
- `summary.dead_letter_records_total`、`summary.replay_history_total` 查看过滤后的完整总数
- `health.status`、`health.recovery_window_active`、`health.persistent_failures_detected` 查看 broker 是否处于恢复窗口或持续失败状态

另外还可以通过以下接口拿到稳定的 runbook catalog：

- `GET /api/v1/runtime/management/runbooks`

如果 CI 失败：

- `full-matrix` 会上传 compose 日志、matrix 启动日志、alert-relay 审计目录
- `light-smoke` 也会上传对应的 compose 日志和 alert-relay 审计目录
- artifact 名称会带上 `run_id / run_attempt / commit sha`，默认保留 14 天

## 6. 测试命令

RabbitMQ 集成测试：

```bash
export NEXUS_RABBITMQ_TEST_URL="amqp://guest:guest@127.0.0.1:5672/%2f"
cargo test -p nexus-runtime rabbitmq -- --nocapture
```

NATS 集成测试：

```bash
export NEXUS_NATS_TEST_URL="nats://127.0.0.1:4222"
cargo test -p nexus-runtime nats -- --nocapture
```
