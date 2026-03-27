# Runtime Worker Group 与 RabbitMQ 联调文档

## 1. 文档目的

本文档记录当前 `nexus-runtime` 在以下两方面的运行方式：

- worker group 如何配置
- RabbitMQ 本地联调与集成测试如何运行

本文档基于当前仓库实现编写，适用于当前阶段的开发与联调。

---

## 2. 当前实现概览

当前 runtime 已支持：

- 按 `queue/lane` 路由任务
- 按 worker group 消费特定 route
- RabbitMQ publish / reserve / ack / retry / reject / dead-letter / replay
- 本地无 RabbitMQ 时跳过 RabbitMQ 集成测试

默认 worker group 为：

- `oj-fast` -> `oj_judge:fast`
- `oj-normal` -> `oj_judge:normal`
- `oj-heavy` -> `oj_judge:heavy`
- `oj-special` -> `oj_judge:special`

---

## 3. Worker Group 配置

## 3.1 环境变量

使用环境变量：

`NEXUS_RUNTIME_WORKER_GROUPS`

格式为：

`group_name=queue:lane[,queue:lane...];group_name=queue:lane[...]`

也就是：

- 不同 worker group 之间用 `;` 分隔
- 同一个 worker group 内多个 binding 用 `,` 分隔
- 单个 binding 用 `queue:lane` 表示

---

## 3.2 示例

### 最小示例

```bash
export NEXUS_RUNTIME_WORKER_GROUPS="oj-fast=oj_judge:fast;oj-special=oj_judge:special"
```

表示：

- 一个 worker group 消费 `oj_judge:fast`
- 一个 worker group 消费 `oj_judge:special`

### 一个 group 吃多个 route

```bash
export NEXUS_RUNTIME_WORKER_GROUPS="oj-main=oj_judge:fast,oj_judge:normal;oj-heavy=oj_judge:heavy;oj-special=oj_judge:special"
```

表示：

- `oj-main` 同时消费 `fast` 和 `normal`
- `oj-heavy` 单独消费 `heavy`
- `oj-special` 单独消费 `special`

---

## 3.3 未配置时的默认值

如果未设置 `NEXUS_RUNTIME_WORKER_GROUPS`，系统会使用内置默认值：

```text
oj-fast=oj_judge:fast
oj-normal=oj_judge:normal
oj-heavy=oj_judge:heavy
oj-special=oj_judge:special
```

---

## 4. RabbitMQ 本地启动

最简单的方式是直接用 Docker。

```bash
docker run -d \
  --name nexus-rabbitmq \
  -p 5672:5672 \
  -p 15672:15672 \
  rabbitmq:3-management
```

启动后：

- AMQP 地址：`amqp://guest:guest@127.0.0.1:5672/%2f`
- 管理后台：`http://127.0.0.1:15672`
- 默认账号密码：`guest / guest`

---

## 5. Runtime / Gateway 本地联调环境变量

## 5.0 进程角色

当前 `nexus-app` 支持通过：

`NEXUS_PROCESS_ROLE`

控制进程角色。

可选值：

- `embedded`
- `gateway`
- `runtime-worker`

含义：

- `embedded`：单进程启动 gateway + runtime worker
- `gateway`：只启动 HTTP/API 入口，不启动后台 worker
- `runtime-worker`：只启动 runtime worker 与运维接口，不启动 OJ/API 入口

如果不设置，默认值为：

`embedded`

前端跨域联调可额外配置：

`NEXUS_CORS_ALLOWED_ORIGINS`

格式为逗号分隔，例如：

```bash
export NEXUS_CORS_ALLOWED_ORIGINS="http://localhost:5173,http://127.0.0.1:5173"
```

如果不设置，`dev` 环境默认允许：

- `http://localhost:5173`
- `http://127.0.0.1:5173`
- `http://localhost:4173`
- `http://127.0.0.1:4173`

节点标识使用：

`NEXUS_RUNTIME_NODE_ID`

如果不设置，默认会生成：

`<HOSTNAME>-<PID>`

---

## 5.1 使用 RabbitMQ 作为 runtime queue backend

```bash
export NEXUS_PROCESS_ROLE=embedded
export NEXUS_RUNTIME_QUEUE_BACKEND=rabbitmq
export NEXUS_RUNTIME_RABBITMQ_URL="amqp://guest:guest@127.0.0.1:5672/%2f"
export NEXUS_RUNTIME_RABBITMQ_EXCHANGE="nexus.runtime"
export NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX="nexus.runtime"
```

## 5.2 配置 worker group

```bash
export NEXUS_RUNTIME_WORKER_GROUPS="oj-fast=oj_judge:fast;oj-normal=oj_judge:normal;oj-heavy=oj_judge:heavy;oj-special=oj_judge:special"
```

## 5.3 运行 gateway

```bash
cargo run -p nexus-gateway
```

如果使用 `nexus-app` 按角色运行，推荐：

### 单进程嵌入式运行

```bash
export NEXUS_PROCESS_ROLE=embedded
cargo run -p nexus-app
```

### 只起 gateway

```bash
export NEXUS_PROCESS_ROLE=gateway
cargo run -p nexus-app
```

### 只起 runtime worker

```bash
export NEXUS_PROCESS_ROLE=runtime-worker
cargo run -p nexus-app
```

此时进程会启动一个轻量 HTTP 服务，绑定：

`NEXUS_BIND_ADDR`

可用于：

- 健康检查
- 节点状态查询
- 集群节点列表查询
- runtime 队列统计
- worker group 查询
- dead-letter 运维接口

---

## 6. RabbitMQ 集成测试

## 6.1 环境变量

RabbitMQ 集成测试使用：

`NEXUS_RABBITMQ_TEST_URL`

例如：

```bash
export NEXUS_RABBITMQ_TEST_URL="amqp://guest:guest@127.0.0.1:5672/%2f"
```

---

## 6.2 运行方式

只跑 runtime：

```bash
cargo test -p nexus-runtime
```

或静默方式：

```bash
cargo test -q -p nexus-runtime
```

---

## 6.3 当前测试覆盖

当前 RabbitMQ 集成测试已覆盖：

- `publish -> reserve -> ack`
- `publish -> reserve -> retry -> reserve`
- `retry 到最大次数后进入 dead-letter`
- `dead-letter -> replay -> reserve`

测试文件位置：

`crates/nexus-runtime/tests/rabbitmq_integration.rs`

---

## 6.5 Worker-only 进程可观测接口

当使用：

`NEXUS_PROCESS_ROLE=runtime-worker`

启动时，可使用以下接口：

- `GET /healthz`
- `GET /api/v1/system/health`
- `GET /api/v1/runtime/node`
- `GET /api/v1/runtime/worker-groups`
- `GET /api/v1/runtime/queues/stats`
- `GET /api/v1/runtime/queues/dead-letters`
- `POST /api/v1/runtime/queues/dead-letters/:delivery_id/replay`

说明：

- `runtime/node` 用于查看当前节点标识、启动时间、worker group
- `worker-groups` 用于确认当前节点到底消费哪些 `queue/lane`
- `queues/stats` 用于观察 broker 队列堆积与租约状态
- `dead-letters` 用于人工排查或回放任务
- 上述三个 GET 接口当前都支持可选过滤参数：
  - `queue`
  - `lane`
  - `group`

---

## 6.7 Gateway 侧 runtime 集群视图

当 gateway 可访问 Redis 时，可通过以下接口查看当前活跃 runtime 节点：

- `GET /api/v1/runtime/nodes`
- `GET /api/v1/runtime/nodes/summary`

返回值来自 Redis 中的：

- `runtime_nodes:<node_id>`

gateway 会聚合所有当前仍在 TTL 内的节点心跳，并返回一个节点列表。

当前这两个 gateway 接口都支持可选过滤参数：

- `queue`
- `lane`
- `group`

其中：

- `nodes` 返回过滤后的活跃节点列表
- `nodes/summary` 返回聚合后的集群摘要，包括节点数、`healthy/stale` 数量、worker group 数、以及 route 覆盖情况

当前节点注册表字段重点包括：

- `node_id`
- `started_at_ms`
- `last_heartbeat_ms`
- `node_status`
- `worker_groups`

当前健康度判定规则：

- Redis 心跳间隔：10 秒
- Redis key TTL：30 秒
- gateway 将 `last_heartbeat_ms` 超过 20 秒未更新的节点标记为 `stale`
- 超过 TTL 的节点会直接从节点列表中消失，而不是继续保留为 `stale`

这意味着：

- `runtime-worker` 看的是“我是谁”
- `gateway` 看的是“现在集群里有哪些活跃节点，以及哪些 route 正在被覆盖”
- `gateway` 角色不再暴露单节点 runtime 调度/死信/任务快照接口；这些接口只在 `embedded` 或 `runtime-worker` 角色可用

---

## 6.6 Redis 节点心跳

当进程角色为：

- `embedded`
- `runtime-worker`

时，系统会周期性向 Redis 写入 runtime 节点心跳。

当前约定：

- Redis key：`runtime_nodes:<node_id>`
- Redis channel：`runtime_node_heartbeats`
- 心跳周期：10 秒
- key TTL：30 秒

写入内容包括：

- `node_id`
- `started_at_ms`
- `worker_groups`

这为后续的 runtime 节点列表、调度面板、集群观测提供基础数据。

---

## 6.4 未设置测试环境变量时的行为

如果没有设置 `NEXUS_RABBITMQ_TEST_URL`：

- 测试不会失败
- 测试会直接返回
- 适合日常本地无 RabbitMQ 的开发场景

这意味着：

- CI 或联调环境中设置该变量即可启用真实 RabbitMQ 测试
- 普通本地开发不会被强制依赖 RabbitMQ

---

## 7. 当前建议的开发节奏

推荐日常开发这样分：

1. 普通代码开发时，直接跑：

```bash
cargo test -q
```

2. 涉及 RabbitMQ 改动时，先启动本地 RabbitMQ，再设置：

```bash
export NEXUS_RABBITMQ_TEST_URL="amqp://guest:guest@127.0.0.1:5672/%2f"
```

然后跑：

```bash
cargo test -q -p nexus-runtime
```

3. 联调 gateway/runtime 路径时，再设置：

```bash
export NEXUS_RUNTIME_QUEUE_BACKEND=rabbitmq
export NEXUS_RUNTIME_RABBITMQ_URL="amqp://guest:guest@127.0.0.1:5672/%2f"
export NEXUS_RUNTIME_WORKER_GROUPS="oj-fast=oj_judge:fast;oj-normal=oj_judge:normal;oj-heavy=oj_judge:heavy;oj-special=oj_judge:special"
```

---

## 8. 当前限制

当前 RabbitMQ 集成测试已经是真实 broker 测试，但仍有两个现实限制：

1. 测试依赖本地或外部 RabbitMQ 可用
2. worker group 目前仍由 gateway 装配时注入，还没有单独的 runtime 节点进程配置体系

这两个限制都不影响当前开发推进，但后续如果拆独立 runtime worker 进程，需要再把 worker group 装配逻辑抽出来。
