# Context

## 当前阶段

当前项目已经进入：

`多 Broker 平台底座稳定 + 统一 Job 平台接口第一阶段已基本验收通过`

当前最重要的上下文是：

- 多 Broker 底座已经完成 P0 与 P1 收口
- 统一 Job 平台接口已经形成第一版稳定边界
- OJ 已经成为第一批真实接入对象
- 下一阶段应该继续让云函数和博客异步能力接入 Job 平台

---

## 已完成的关键事项

## 1. 多 Broker 平台底座

当前 `nexus-runtime` 已支持：

- `RabbitMQ`
- `NATS JetStream`
- `Redis Streams`
- `memory`

核心目录：

- [mod.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/mod.rs)
- [rabbitmq.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/rabbitmq.rs)
- [nats.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/nats.rs)
- [redis.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/redis.rs)

已经具备：

- enqueue / reserve / ack / retry / reject
- dead-letter / replay
- stats
- reclaim / recovery
- capability profile
- contract tests

---

## 2. P0 与 P1

P0 已收口：

- 观测
- 告警
- 审计
- 部署配置
- 故障演练
- CI matrix

P1 已收口：

- runtime 管理视图
- broker health / degradation
- alerts / recommended_action / runbook_ref
- dead-letter / replay 排障接口

关键文档：

- [P0_生产稳定性与运维闭环_Runbook.md](/home/fmy/NexusCore/P0_生产稳定性与运维闭环_Runbook.md)
- [P1_阶段收口清单.md](/home/fmy/NexusCore/P1_阶段收口清单.md)
- [Runtime_Gateway_运维_API_契约文档.md](/home/fmy/NexusCore/Runtime_Gateway_运维_API_契约文档.md)

---

## 3. Job 平台当前状态

新增独立 crate：

- [Cargo.toml](/home/fmy/NexusCore/crates/nexus-jobs/Cargo.toml)

当前模块划分：

- [model](/home/fmy/NexusCore/crates/nexus-jobs/src/model)
- [api](/home/fmy/NexusCore/crates/nexus-jobs/src/api)
- [handlers](/home/fmy/NexusCore/crates/nexus-jobs/src/handlers)
- [runtime](/home/fmy/NexusCore/crates/nexus-jobs/src/runtime)
- [domains](/home/fmy/NexusCore/crates/nexus-jobs/src/domains)
- [router.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/router.rs)

当前已经具备：

- Job 核心对象模型
- Job 生命周期映射
- Job 提交与校验
- Job 查询与管理接口
- Job 历史事件模型
- Job handler 注册与执行契约
- Job -> Runtime 的适配

---

## 4. OJ 接入现状

OJ 已经通过 Job 平台主链提交任务，而不是直接依赖 runtime queue。

关键文件：

- [application.rs](/home/fmy/NexusCore/crates/nexus-oj/src/application.rs)
- [lib.rs](/home/fmy/NexusCore/crates/nexus-oj/src/lib.rs)
- [oj.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/domains/oj.rs)

当前 OJ 已实现：

- 从 submission 构建 `JobDefinition`
- 通过 `JobPlatformService.submit(...)` 提交
- 由 OJ handler 转成 runtime dispatch
- 通过 Job API 查看状态与历史

---

## 5. Job 平台当前对控制面可见的信息

当前 Job API 已有：

- `GET /api/v1/jobs`
- `GET /api/v1/jobs/:job_id`
- `GET /api/v1/jobs/:job_id/history`
- `GET /api/v1/jobs/handlers`
- `GET /api/v1/jobs/management/overview`

当前 JobSnapshot 已可见：

- job_id
- job_type
- origin
- dispatch
- status
- error
- history
- handler descriptor
- latest submission failure

当前管理摘要已可见：

- total_jobs
- queued
- running
- succeeded
- retrying
- dead_lettered
- failed
- submission_rejected

---

## 6. Job handler 契约现状

当前 handler 契约已包含：

- `JobHandlerDescriptor`
- `JobHandlerCapabilities`
- `JobExecutionContext`
- `JobDispatchPlan`
- `JobHandlerResult`
- `JobHandlerFailure`

当前支持的语义：

- dispatch 到 runtime
- handler rejection
- handler 能力声明

这意味着平台已经能表达：

- 谁处理某种 Job
- handler 需要什么能力
- handler 是不是在进入 runtime 前拒绝了任务

---

## 7. 已补齐的设计文档

当前和统一 Job 平台接口验收直接相关的文档包括：

- [统一Job_平台接口设计验收标准文档.md](/home/fmy/NexusCore/统一Job_平台接口设计验收标准文档.md)
- [Job平台_业务模块统一映射说明.md](/home/fmy/NexusCore/Job平台_业务模块统一映射说明.md)
- [Job平台_Runtime_Broker_职责边界文档.md](/home/fmy/NexusCore/Job平台_Runtime_Broker_职责边界文档.md)
- [多Broker_平台底座_阶段性总结.md](/home/fmy/NexusCore/多Broker_平台底座_阶段性总结.md)

---

## 8. 当前推荐的下一步

如果继续推进，建议优先做：

1. 云函数模块的第一版 Job domain 与 handler
2. 博客异步任务的第一版 Job domain 与 handler
3. Job 查询/管理接口的更强过滤与聚合
4. Job 控制面的第一版设计

---

## 9. 当前不要做的事

当前不建议把重心再拉回：

- 继续无限扩张 broker 底层细节
- 提前做完整前端产品设计
- 提前做完整 RBAC / 多租户控制台
- 提前做复杂 DAG / 工作流系统

当前更合理的方向是：

`让统一 Job 平台真正成为业务模块的统一接入面。`
