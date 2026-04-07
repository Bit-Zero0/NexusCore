# Runtime/Gateway 运维 API 契约文档

本文档描述当前 `runtime-worker` 与 `gateway` 已实现的运维与调度相关 HTTP API 契约。

范围说明：
- 仅覆盖当前仓库中已经实现的运维接口。
- 不覆盖 OJ 业务 REST 接口。
- 不覆盖未来规划中的控制台功能、权限控制、告警系统。

相关实现：
- runtime 路由：[router.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-runtime/src/router.rs)
- runtime 状态模型：[executor.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-runtime/src/executor.rs)
- gateway 聚合接口：[lib.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-gateway/src/lib.rs)
- 运行模式与心跳写入：[main.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-app/src/main.rs)

## 1. 基本约定

Base Path:

```text
/api/v1
```

Content-Type:

```text
application/json
```

错误响应格式：

```json
{
  "code": "NOT_FOUND",
  "message": "not found: runtime task not found: task_xxx"
}
```

共享错误码来源于 [error.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-shared/src/error.rs)：
- `BAD_REQUEST`
- `DATABASE_ERROR`
- `INVALID_CONFIG`
- `UNAUTHORIZED`
- `NOT_FOUND`
- `INTERNAL_ERROR`

## 2. 接口分层

当前平台运维接口分两层：

1. `runtime-worker` 单节点接口  
用途：查看当前节点自身状态、当前消费的 worker group、队列统计、死信、单任务快照。

2. `gateway` 集群聚合接口  
用途：查看整个 runtime 集群当前有哪些活跃节点、节点是否 stale、route 覆盖是否完整。

角色边界：
- `embedded`：同时暴露 gateway 接口和 runtime 单节点接口
- `gateway`：只暴露 gateway 聚合接口，不暴露 runtime 单节点接口
- `runtime-worker`：只暴露 runtime 单节点接口与健康检查

## 3. 健康与节点状态约定

### 3.1 RuntimeNodeHealthStatus

序列化为 `snake_case`：

- `healthy`
- `stale`

### 3.2 stale 判定规则

当前实现规则：

- runtime 心跳写 Redis 的周期是 10 秒
- Redis key TTL 是 30 秒
- gateway 将 `last_heartbeat_ms` 超过 20 秒未更新的节点标记为 `stale`
- 超过 TTL 的节点会直接从 `runtime_nodes:*` 中消失，而不是继续显示为 `stale`

### 3.3 RuntimeNodeStatus

```json
{
  "node_id": "runtime-host-1234",
  "started_at_ms": 1710000000000,
  "last_heartbeat_ms": 1710000010000,
  "node_status": "healthy",
  "worker_groups": [
    {
      "name": "oj-fast",
      "bindings": [
        {
          "queue": "oj_judge",
          "lane": "fast"
        }
      ]
    }
  ]
}
```

字段说明：
- `node_id`: 节点标识，默认格式为 `<HOSTNAME>-<PID>`，也可通过环境变量覆盖
- `started_at_ms`: 节点启动时间
- `last_heartbeat_ms`: 最近一次心跳时间
- `node_status`: gateway 侧判定后的健康状态
- `worker_groups`: 当前节点启动的消费分组

## 4. Health 接口

前端跨域联调说明：
- 可通过 `NEXUS_CORS_ALLOWED_ORIGINS` 配置允许的前端来源
- 格式为逗号分隔，例如 `http://localhost:5173,http://127.0.0.1:5173`
- `dev` 环境默认允许 `localhost/127.0.0.1` 的 `5173` 和 `4173` 端口

### 4.1 Gateway 健康检查

```http
GET /healthz
GET /api/v1/system/health
```

响应：

```json
{
  "status": "ok",
  "service": "nexus-gateway",
  "version": "..."
}
```

### 4.2 Runtime Worker 健康检查

当 `NEXUS_PROCESS_ROLE=runtime-worker` 时：

```http
GET /healthz
GET /api/v1/system/health
```

响应：

```json
{
  "status": "ok",
  "service": "nexus-runtime-worker",
  "version": "..."
}
```

## 5. Runtime Worker 单节点接口

这些接口由 `runtime-worker` 进程直接暴露，也会在 `embedded` 模式下随 gateway 一并挂出。

### 5.1 获取当前节点状态

```http
GET /api/v1/runtime/node
```

响应：`RuntimeNodeStatus`

用途：
- 查看当前节点身份
- 确认启动时间
- 确认当前加载的 worker groups

### 5.2 获取当前节点的 worker groups

```http
GET /api/v1/runtime/worker-groups
```

可选查询参数：
- `queue`
- `lane`
- `group`

响应：

```json
[
  {
    "name": "oj-fast",
    "bindings": [
      {
        "queue": "oj_judge",
        "lane": "fast"
      }
    ]
  }
]
```

过滤语义：
- `group` 按 worker group 名称精确匹配
- `queue/lane` 按任一 binding 精确匹配

### 5.3 获取当前节点队列统计

```http
GET /api/v1/runtime/queues/stats
```

可选查询参数：
- `queue`
- `lane`
- `group`

响应：

```json
[
  {
    "queue": "oj_judge",
    "lane": "fast",
    "queued": 3,
    "leased": 1,
    "dead_lettered": 0
  }
]
```

字段说明：
- `queued`: 当前等待消费的任务数
- `leased`: 已经被 worker 取走、尚未 ack 的任务数
- `dead_lettered`: 当前 route 的死信数量

说明：
- 当前 `stats` 实际按 `queue/lane` 过滤
- `group` 参数保留在统一查询模型里，但不会额外改变队列统计聚合逻辑

### 5.4 获取死信列表

```http
GET /api/v1/runtime/queues/dead-letters
```

可选查询参数：
- `queue`
- `lane`
- `group`

响应：

```json
[
  {
    "delivery_id": "dlv_001",
    "task_id": "task_sub_001",
    "queue": "oj_judge",
    "lane": "fast",
    "attempt": 3,
    "error": "runtime execution failed: ...",
    "dead_lettered_at": 1710000012345,
    "task": {
      "task_id": "task_sub_001",
      "task_type": "oj_judge",
      "source_domain": "oj",
      "source_entity_id": "sub_001",
      "queue": "oj_judge",
      "lane": "fast",
      "retry_policy": {
        "max_attempts": 3,
        "retry_delay_ms": 1000
      },
      "payload": {
        "kind": "oj_judge",
        "submission_id": "sub_001",
        "problem_id": "two-sum",
        "user_id": "u1",
        "language": "cpp",
        "judge_mode": "acm",
        "source_code": "int main() { return 0; }",
        "limits": {
          "time_limit_ms": 1000,
          "memory_limit_kb": 262144
        },
        "testcases": [],
        "judge_config": null
      }
    }
  }
]
```

用途：
- 查看死信任务来源
- 结合 `task` 做人工分析和回放

### 5.5 回放死信任务

```http
POST /api/v1/runtime/queues/dead-letters/:delivery_id/replay
```

响应：`RuntimeQueueReceipt`

```json
{
  "task_id": "task_sub_001",
  "queue": "oj_judge",
  "lane": "fast",
  "status": "queued"
}
```

语义：
- 指定 `delivery_id` 的死信记录会从死信列表移除
- 原任务重新入队
- 返回新的排队回执

### 5.6 调度任务

```http
POST /api/v1/runtime/tasks/schedule
```

请求体：`RuntimeTask`

响应：`RuntimeQueueReceipt`

用途：
- 将任务真正送入 runtime queue
- 返回排队后的 `queue/lane/status`

### 5.7 模拟任务

```http
POST /api/v1/runtime/tasks/simulate
```

请求体：`RuntimeTask`

响应：`RuntimeSimulationReport`

```json
{
  "execution_id": "rt_001",
  "task_id": "task_sub_001",
  "status": "simulated",
  "profile": {
    "language": "cpp",
    "judge_mode": "acm",
    "testcase_count": 2,
    "total_score": 100,
    "time_limit_ms": 1000,
    "memory_limit_kb": 262144
  },
  "plan": {
    "language": "cpp",
    "compile_required": true,
    "source_filename": "main.cpp",
    "executable_filename": "main",
    "compile_command": ["g++", "..."],
    "run_command": ["./main"],
    "sandbox_profile": "nsjail",
    "seccomp_policy": "cpp_default",
    "readonly_mounts": []
  },
  "case_results": [],
  "message": "runtime simulation completed; task is ready for real execution"
}
```

用途：
- 联调语言运行计划
- 联调编译与沙箱配置
- 在正式调度前确认 task 合法性

### 5.8 获取任务快照

```http
GET /api/v1/runtime/tasks/:task_id
```

响应：`RuntimeTaskSnapshot`

```json
{
  "task_id": "task_sub_001",
  "source_domain": "oj",
  "queue": "oj_judge",
  "lane": "fast",
  "status": "running",
  "message": "interpreter task started",
  "artifacts": null,
  "outcome": null,
  "error": null
}
```

### 5.9 RuntimeTaskLifecycleStatus

序列化为 `snake_case`：

- `queued`
- `retrying`
- `preparing`
- `prepared`
- `compiling`
- `running`
- `completed`
- `failed`
- `dead_lettered`

## 6. Gateway 集群聚合接口

这些接口面向平台入口层，依赖 Redis 中的 runtime 节点心跳数据。

### 6.1 获取活跃节点列表

```http
GET /api/v1/runtime/nodes
```

可选查询参数：
- `queue`
- `lane`
- `group`
- `status`

其中 `status` 当前支持：
- `healthy`
- `stale`

响应：`RuntimeNodeStatus[]`

说明：
- 只返回当前 Redis TTL 内仍然存在的节点
- gateway 在返回前会重新计算 `node_status`
- 返回结果按 `node_id` 排序

### 6.2 获取集群摘要

```http
GET /api/v1/runtime/nodes/summary
```

可选查询参数：
- `queue`
- `lane`
- `group`
- `status`

响应：

```json
{
  "total_nodes": 2,
  "total_worker_groups": 3,
  "healthy_nodes": 2,
  "stale_nodes": 0,
  "routes": [
    {
      "queue": "oj_judge",
      "lane": "fast",
      "node_count": 2,
      "worker_group_count": 2
    },
    {
      "queue": "oj_judge",
      "lane": "special",
      "node_count": 1,
      "worker_group_count": 1
    }
  ],
  "groups": [
    {
      "name": "oj-fast",
      "node_count": 2,
      "binding_count": 1
    },
    {
      "name": "oj-special",
      "node_count": 1,
      "binding_count": 1
    }
  ]
}
```

字段说明：
- `total_nodes`: 当前过滤条件下的节点数
- `total_worker_groups`: 所有节点上 worker group 数量之和
- `healthy_nodes`: 当前过滤结果中的 healthy 节点数
- `stale_nodes`: 当前过滤结果中的 stale 节点数
- `routes`: 当前集群对每个 `queue/lane` 的覆盖情况
- `groups`: 当前各 worker group 的节点覆盖情况

用途：
- 判断某条 route 是否只有单节点覆盖
- 判断 `oj-fast / oj-special` 是否缺节点
- 为运维面板直接提供摘要数据

## 7. Redis 注册表约定

当前 runtime 节点注册信息写入 Redis：

Key:

```text
runtime_nodes:<node_id>
```

Channel:

```text
runtime_node_heartbeats
```

载荷内容：`RuntimeNodeStatus`

写入逻辑位于 [main.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-app/src/main.rs)。

## 8. 当前已稳定的过滤参数

统一查询参数：
- `queue`
- `lane`
- `group`

节点注册表额外支持：
- `status`

当前过滤语义均为精确匹配，不支持：
- 模糊匹配
- 多值查询
- 前缀匹配
- 正则匹配

## 9. 当前不在本契约范围内的内容

以下内容暂不写入当前稳定运维 API 契约：
- 节点摘除
- 节点优雅下线
- 节点权重与调度优先级控制
- 手动触发 worker group 扩缩容
- WebSocket 运维推送
- 鉴权与 RBAC

这些能力后续若补充，建议以“平台运维控制 API”单独成文，不直接混入当前只读/诊断型契约。
### 5.4 获取 broker 管理视图

```http
GET /api/v1/runtime/management/broker
```

可选查询参数：
- `queue`
- `lane`
- `group`
- `task_id`
- `delivery_id`
- `limit`
- `offset`

用途：
- 给控制面/UI 提供单个稳定的 broker 管理视图
- 汇总 broker 能力、worker groups、queue stats、dead letters、replay history

返回要点：
- `summary.dead_letter_records_total` 表示过滤后的 dead-letter 总数，不受当前分页窗口影响
- `summary.replay_history_total` 表示过滤后的 replay history 总数，不受当前分页窗口影响
- `health.status` 统一表达 broker 当前是否处于 `healthy` / `degraded`
- `health.degradation_reasons` 给出结构化降级原因，控制面不需要自己根据布尔值二次推断
- `health.alerts` 给出可直接渲染的告警摘要，包含 `code / severity / reason / message / recommended_action`
- `health.alerts[].recommended_action` 采用结构化字段，当前包含 `label / action_kind / runbook_ref`
- `health.alerts[].recommended_action.runbook` 给出可直接跳转的 runbook 元信息，包含 `runbook_ref / title / doc_path / section_ref`
- `health.recovery_window_active` 表示 broker 最近是否仍处于恢复窗口
- `health.persistent_failures_detected` 表示最近失败计数是否达到持续失败阈值
- `health.last_failure_at_ms` 和 `health.recent_failure_count` 用于控制面做恢复态展示和排障跳转

### 5.5 获取 broker runbook catalog

```http
GET /api/v1/runtime/management/runbooks
```

用途：
- 给控制面提供稳定的 runbook 引用目录
- 让 `runbook_ref` 可以解析成 `title / doc_path / section_ref`

### 5.6 获取 dead-letter replay 历史

```http
GET /api/v1/runtime/queues/replays
```

可选查询参数：
- `queue`
- `lane`
- `task_id`
- `delivery_id`
- `limit`
- `offset`

返回结果按 `replayed_at_ms` 倒序排列，默认 `limit=50`。
