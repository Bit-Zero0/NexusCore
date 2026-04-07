# 多 Broker 平台底座与 Job 平台阶段性总结

## 1. 文档目的

本文档用于总结当前项目在以下两个方向上的阶段性进展：

- 多 Broker 平台底座
- 统一 Job 平台接口

重点不是描述理想状态，而是描述当前仓库里已经真实落地的能力、当前所处阶段，以及下一阶段最值得继续推进的事项。

---

## 2. 当前总体判断

当前项目已经从：

`单一运行时队列适配阶段`

推进到了：

`可插拔多 Broker 异步任务底座 + 生产级运维闭环 + 统一 Job 平台接口第一阶段成型`

一句话概括当前状态：

`Broker 层已经可插拔、可观测、可恢复，Job 平台的提交/消费/查询/管理边界也已经成型，当前已经具备继续进入统一 Job 平台落地验证阶段的条件。`

---

## 3. 已完成的核心阶段成果

## 3.1 多 Broker 平台底座已经成型

当前 `nexus-runtime` 已形成统一的 Broker 抽象入口，核心在：

- [mod.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/mod.rs)

当前已完成：

- `BrokerAdapter` 内部抽象
- `RabbitMQ / NATS JetStream / Redis Streams / memory` 四类 backend 路径
- capability profile 显式化
- route catalog / dead-letter / replay workflow 收口
- RabbitMQ 重连恢复
- NATS ack wait / lease recovery
- Redis Streams pending reclaim

这意味着系统已经不再停留在“能不能切换 MQ”，而是进入“如何稳定扩展更多 MQ 和上层平台”的阶段。

---

## 3.2 Broker 语义已经基本稳定

当前统一下来的 broker 语义包括：

- `enqueue`
- `reserve`
- `ack`
- `retry`
- `reject`
- `dead-letter`
- `replay_dead_letter`
- `stats`
- `reclaim / lease recovery`

相关契约测试位于：

- [broker_contract.rs](/home/fmy/NexusCore/crates/nexus-runtime/tests/support/broker_contract.rs)

当前已经覆盖：

- publish -> reserve -> ack
- retry -> dead-letter -> replay
- reject -> dead-letter
- dead-letter 跨重连持久化
- unacked delivery 跨重连 reclaim
- 重复投递收敛
- 延迟重试边界
- 多 route 轮询与公平性
- dead-letter replay 后一致性

这一步保证后续 Job 平台不是建立在脆弱、不一致的消息语义之上。

---

## 3.3 P0 与 P1 已经形成完整底座闭环

P0 已完成：

- 真实告警投递联调
- broker 故障恢复演练
- 配置与部署收口
- 启动矩阵与 smoke 脚本
- CI 分层回归

相关文档和脚本包括：

- [P0_生产稳定性与运维闭环_Runbook.md](/home/fmy/NexusCore/P0_生产稳定性与运维闭环_Runbook.md)
- [生产部署与配置收口手册.md](/home/fmy/NexusCore/生产部署与配置收口手册.md)
- [run_broker_failure_drill.sh](/home/fmy/NexusCore/scripts/run_broker_failure_drill.sh)
- [verify_alert_pipeline.sh](/home/fmy/NexusCore/scripts/verify_alert_pipeline.sh)
- [check_deploy_config.sh](/home/fmy/NexusCore/scripts/check_deploy_config.sh)
- [runtime-matrix.yml](/home/fmy/NexusCore/.github/workflows/runtime-matrix.yml)

P1 已完成：

- Broker 管理视图
- health / degradation / alerts
- recommended_action / runbook_ref
- dead-letter / replay 排障接口
- broker 能力显式化

相关文档包括：

- [P1_阶段收口清单.md](/home/fmy/NexusCore/P1_阶段收口清单.md)
- [Runtime_Gateway_运维_API_契约文档.md](/home/fmy/NexusCore/Runtime_Gateway_运维_API_契约文档.md)

这意味着底座已经不仅是“能跑”，而是“能观测、能告警、能排障、能解释”。

---

## 3.4 监控、告警、审计链路已经成闭环

当前已完成：

- Prometheus 指标统一出口
- Grafana dashboard
- Alertmanager 路由
- Rust 版 `alert-relay`
- dry-run/logger receiver
- 审计日志 JSONL 落盘
- 大小滚动、保留清理
- 查询、导出、聚合脚本

关键文件包括：

- [docker-compose.dev.yml](/home/fmy/NexusCore/docker-compose.dev.yml)
- [README.md](/home/fmy/NexusCore/monitoring/README.md)
- [prometheus.yml](/home/fmy/NexusCore/monitoring/prometheus/prometheus.yml)
- [nexus-runtime-alerts.yml](/home/fmy/NexusCore/monitoring/prometheus/rules/nexus-runtime-alerts.yml)
- [alertmanager.yml](/home/fmy/NexusCore/monitoring/alertmanager/alertmanager.yml)
- [main.rs](/home/fmy/NexusCore/crates/nexus-alert-relay/src/main.rs)
- [query_alert_audit.py](/home/fmy/NexusCore/scripts/query_alert_audit.py)

这意味着系统已经具备比较完整的运维闭环，而不是只依赖日志排障。

---

## 3.5 统一 Job 平台接口第一阶段已经成型

当前新增了独立的 `nexus-jobs` crate，见：

- [Cargo.toml](/home/fmy/NexusCore/crates/nexus-jobs/Cargo.toml)

当前已形成的核心结构包括：

- Job 模型层
- Job 提交与校验接口
- Job 查询与管理接口
- Job 事件与历史模型
- Job handler 注册与执行契约
- Runtime 适配层

关键目录包括：

- [model](/home/fmy/NexusCore/crates/nexus-jobs/src/model)
- [api](/home/fmy/NexusCore/crates/nexus-jobs/src/api)
- [handlers](/home/fmy/NexusCore/crates/nexus-jobs/src/handlers)
- [runtime](/home/fmy/NexusCore/crates/nexus-jobs/src/runtime)
- [domains](/home/fmy/NexusCore/crates/nexus-jobs/src/domains)

---

## 3.6 Job 平台核心对象与生命周期已经落地

当前已经具备：

- `JobDefinition`
- `JobId`
- `JobType`
- `JobPayload`
- `JobRoute`
- `JobRetryPolicy`
- `JobTimeoutPolicy`
- `JobStatus`
- `JobResult`
- `JobFailure`

相关实现位于：

- [job.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/model/job.rs)
- [status.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/model/status.rs)
- [result.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/model/result.rs)

这一步确保 Job 平台已经不只是一个名字，而是有稳定对象模型可供业务模块复用。

---

## 3.7 Job 提交、消费、查询、管理边界已经形成

当前已经具备：

- `JobPlatformService`
- `JobSubmissionValidator`
- `JobQueryService`
- `JobHandler`
- `JobExecutionContext`
- `JobHandlerResult`
- `JobHandlerFailure`

相关实现位于：

- [service.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/api/service.rs)
- [validator.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/api/validator.rs)
- [query.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/api/query.rs)
- [contract.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/handlers/contract.rs)

当前 Job API 已暴露：

- `GET /api/v1/jobs`
- `GET /api/v1/jobs/:job_id`
- `GET /api/v1/jobs/:job_id/history`
- `GET /api/v1/jobs/handlers`
- `GET /api/v1/jobs/management/overview`

相关路由位于：

- [router.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/router.rs)

---

## 3.8 OJ 已成为第一批真实接入对象

当前 OJ 已经不是“只在 build task 时借用一下 Job 模型”，而是已经正式走 Job 平台主链。

相关实现位于：

- [application.rs](/home/fmy/NexusCore/crates/nexus-oj/src/application.rs)
- [lib.rs](/home/fmy/NexusCore/crates/nexus-oj/src/lib.rs)
- [oj.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/domains/oj.rs)

当前 OJ 已经验证了：

- 从 submission 构建 JobDefinition
- 通过 `JobPlatformService` 提交
- 通过 OJ handler 映射到 runtime
- 通过 Job 查询接口查看历史与状态

这意味着统一 Job 平台接口已经不只是设计稿，而是有真实业务接入验证。

---

## 3.9 Job 平台的失败与控制面可见性已经补齐

当前已经支持：

- handler descriptor / capabilities 显式化
- handler rejection 结构化返回
- submission 阶段失败保留为 Job 历史
- 没有 runtime snapshot 的 rejected job 仍然可查询
- Job 管理摘要中区分 `submission_rejected`

相关实现位于：

- [descriptor.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/handlers/descriptor.rs)
- [submitter.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/runtime/submitter.rs)
- [query.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/api/query.rs)
- [management.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/model/management.rs)

这意味着控制面已经可以回答：

- 某个 Job 由哪个 handler 处理
- handler 具备哪些能力
- 某个 Job 是否在进入 runtime 之前就被平台拒绝

---

## 3.10 Job 平台验收缺口已补齐为正式文档

当前已经补齐了两份关键设计文档：

- [Job平台_业务模块统一映射说明.md](/home/fmy/NexusCore/Job平台_业务模块统一映射说明.md)
- [Job平台_Runtime_Broker_职责边界文档.md](/home/fmy/NexusCore/Job平台_Runtime_Broker_职责边界文档.md)

这意味着：

- OJ / 云函数 / 博客 的统一接入方式已经形成正式说明
- Job 平台 / Runtime / Broker 的职责边界已经成文，不再只体现在代码实现里

---

## 4. 当前项目所处阶段

如果按阶段来定义，我会把当前状态称为：

`多 Broker 平台底座已经稳定，统一 Job 平台接口第一阶段已经达到可验收状态，开始进入统一 Job 平台落地验证与第二批业务映射准备阶段。`

这说明当前工作的重心已经从：

- “把底座做出来”

转向：

- “让 Job 平台逐步成为 OJ / 云函数 / 博客的统一接入面”

---

## 5. 当前已经为哪些未来模块铺路

## 5.1 对云函数模块的价值

当前已经提前准备好的能力包括：

- 统一 Job 提交入口
- handler 契约与执行上下文
- 失败重试、dead-letter、replay
- reclaim / recovery
- 监控、告警、管理视图

这意味着云函数后续主要需要补的是：

- 函数模型
- 触发器
- 资源配额与权限
- 函数执行结果模型

底层调度与恢复骨架已经具备。

---

## 5.2 对博客模块的价值

博客模块不会整体建立在 Job 平台上，但它的异步能力已经有清晰落点，例如：

- 搜索索引
- 内容审核
- 通知发送
- 静态化生成
- 媒体处理

这意味着博客不需要再单独造一套异步基础设施。

---

## 6. 当前还没有完成什么

虽然统一 Job 平台接口第一阶段已经基本达成验收标准，但仍有几类工作尚未开始或尚未完成：

## 6.1 统一 Job 平台的第二批业务接入尚未开始

当前只有 OJ 完成了真实接入。

尚未正式开始：

- 云函数 handler 与 job domain
- 博客异步任务 handler 与 job domain

---

## 6.2 Job 结果模型仍偏基础

当前已经有 `JobResult / JobFailure`，但还没有形成更完整的：

- 业务结果存储策略
- result payload 统一建模
- 结果权限与可见性边界

---

## 6.3 Job 控制面仍停留在 API 级

当前已经有足够的 Job API 与 runtime 管理视图，但尚未开始：

- Job 控制台界面
- 面向业务模块的 Job 管理操作面板
- Job 与 runtime 管理视图的统一前端编排

---

## 7. 下一阶段最值得继续推进的方向

建议按这个顺序继续：

1. 为云函数设计第一版 Job domain 与 handler
2. 为博客异步任务设计第一版 Job domain 与 handler
3. 继续增强 Job 查询/管理接口的过滤、聚合与控制面友好性
4. 开始设计面向上层模块的统一 Job 控制面

---

## 8. 一句话结论

当前项目已经从“多 Broker 基础设施建设”推进到了“多 Broker 底座稳定 + 统一 Job 平台接口可验收”的阶段。

下一步最重要的不是继续堆底层能力，而是让：

`OJ / 云函数 / 博客逐步通过同一套 Job 平台接口接入系统。`
