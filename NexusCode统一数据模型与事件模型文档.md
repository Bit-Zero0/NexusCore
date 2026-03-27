# NexusCode 统一数据模型与事件模型文档

## 1. 文档目标

本文档定义 `NexusCode` 平台的统一数据模型思想与统一事件模型设计原则。

文档的目标不是直接给出数据库建表 SQL，而是回答以下问题：

- 平台中有哪些核心实体
- 哪些实体属于平台公共层，哪些属于业务域
- 数据模型应该如何分层
- 跨模块之间应该传什么，不应该传什么
- 平台事件应该如何定义
- 为什么要尽早定义统一事件模型

本文档是未来数据库设计、接口设计、实时消息设计和模块边界设计的上层依据。

---

## 2. 统一数据模型设计目标

NexusCode 不是单一业务系统，因此数据模型不能只围绕 OJ 或博客来设计。

统一数据模型的目标是：

### 2.1 建立平台公共语言

无论是 OJ、博客、函数还是笔记同步，都需要共享一套平台公共概念，例如：

- 用户
- 资源
- 发布记录
- 执行任务
- 事件

### 2.2 避免跨域耦合

平台内各业务域应有自己的领域模型，但不应互相直接侵入内部结构。

### 2.3 支撑未来服务化

如果现在的数据和事件模型就足够清晰，后续从模块化单体拆为服务时，改造成本会更低。

### 2.4 支撑实时消息和异步任务

NexusCode 不只是 CRUD 平台，还包含：

- 实时推送
- 异步执行
- 发布流程
- 调度状态

因此必须有明确的统一事件模型。

---

## 3. 数据模型分层

建议将平台数据模型分为四层：

### 3.1 平台基础模型

所有业务域共享的基础实体。

### 3.2 业务领域模型

具体业务域的核心实体。

### 3.3 执行模型

与任务执行、日志、资源控制相关的模型。

### 3.4 事件模型

与系统状态变化、消息传播、实时通知相关的模型。

---

## 4. 平台基础模型

## 4.1 User

### 定义

平台用户。

### 作用

- 身份主体
- 权限主体
- 内容创建者
- 提交发起者
- 函数调用者

### 关键属性

- `user_id`
- `username`
- `display_name`
- `email`
- `avatar_url`
- `status`
- `created_at`
- `updated_at`

---

## 4.2 Session / Token

### 定义

用户登录态和身份凭证。

### 作用

- Web 会话
- API 调用鉴权
- 客户端登录
- 发布操作授权

### 关键属性

- `session_id`
- `user_id`
- `token_type`
- `issued_at`
- `expired_at`
- `scopes`

---

## 4.3 ResourceIdentity

### 定义

平台中所有重要对象的统一资源标识抽象。

### 作用

用于统一表示：

- 题目
- 文档
- 函数
- 提交
- 发布对象

### 关键属性

- `resource_type`
- `resource_id`
- `owner_id`
- `visibility`

---

## 4.4 Tag

### 定义

统一标签模型。

### 用途

- 题目标签
- 文档标签
- 博客标签
- 函数分类标签

### 说明

标签可以统一建模，但不应强迫不同业务域共享完全相同的业务解释。

---

## 4.5 Asset

### 定义

平台资源文件抽象。

### 用途

- 图片
- 附件
- 题目资源
- 发布静态资源
- 文档嵌入资源

### 关键属性

- `asset_id`
- `storage_key`
- `mime_type`
- `size_bytes`
- `owner_id`
- `created_at`

---

## 5. 内容域模型

## 5.1 Document

### 定义

统一内容文档实体。

### 作用

它是博客、笔记、知识页、题解等内容形态的基础。

### 关键属性

- `document_id`
- `title`
- `slug`
- `author_id`
- `content_raw`
- `content_ast_ref`
- `status`
- `visibility`
- `created_at`
- `updated_at`

---

## 5.2 Block

### 定义

文档块级结构。

### 用途

- 支撑块引用
- 支撑双链
- 支撑未来块级编辑
- 支撑细粒度渲染

### 关键属性

- `block_id`
- `document_id`
- `block_type`
- `content`
- `order_index`

---

## 5.3 Link / Backlink

### 定义

文档之间的连接关系。

### 用途

- 双链
- 反向链接
- 知识网络

### 关键属性

- `source_document_id`
- `target_document_id`
- `source_block_id`
- `link_type`

---

## 5.4 PublishRecord

### 定义

内容发布记录。

### 用途

- 文档发布为博客文章
- 发布历史跟踪
- 发布目标追踪

### 关键属性

- `publish_id`
- `document_id`
- `publish_target`
- `version`
- `published_at`
- `published_by`

---

## 6. OJ 域模型

## 6.1 Problem

### 定义

OJ 题目主体。

### 关键属性

- `problem_id`
- `problem_code`
- `title`
- `judge_mode`
- `difficulty`
- `is_published`
- `author_id`
- `created_at`
- `updated_at`

### 特殊说明

`judge_mode` 用于区分：

- ACM
- Functional
- EasyJudge

---

## 6.2 ProblemStatement

### 定义

题面内容模型。

### 关键属性

- `statement_md`
- `input_desc_md`
- `output_desc_md`
- `samples`
- `notes`

---

## 6.3 ProblemLimits

### 定义

题目资源限制模型。

### 作用

按语言区分时间和内存限制。

### 关键属性

- `problem_id`
- `language`
- `time_limit_ms`
- `memory_limit_kb`

---

## 6.4 Testcase

### 定义

测试用例模型。

### 关键属性

- `testcase_id`
- `problem_id`
- `case_no`
- `input_data`
- `expected_output`
- `is_sample`
- `score`
- `extra_config`

---

## 6.5 JudgeMethodConfig

### 定义

题目判题方式配置。

### 可包含内容

- 普通严格比对
- Validator 配置
- SPJ 配置
- EasyJudge 元数据
- Functional 签名

### 关键属性

- `judge_method`
- `validator_config`
- `spj_config`
- `easy_config`
- `function_signature`

---

## 6.6 Submission

### 定义

用户一次提交记录。

### 关键属性

- `submission_id`
- `problem_id`
- `user_id`
- `language`
- `source_code`
- `status`
- `score`
- `max_score`
- `route_lane`
- `created_at`
- `updated_at`

---

## 6.7 SubmissionResult

### 定义

提交结果总览模型。

### 关键属性

- `submission_id`
- `overall_status`
- `compile_output`
- `runtime_output`
- `time_used_ms`
- `memory_used_kb`
- `judge_summary`

---

## 6.8 SubmissionTestcaseResult

### 定义

测试点明细结果。

### 关键属性

- `submission_id`
- `testcase_id`
- `case_no`
- `status`
- `score`
- `time_used_ms`
- `memory_used_kb`
- `actual_output`
- `expected_output_snapshot`
- `message`

---

## 7. Function 域模型

## 7.1 FunctionDefinition

### 定义

云函数定义主体。

### 关键属性

- `function_id`
- `name`
- `slug`
- `owner_id`
- `runtime`
- `entrypoint`
- `visibility`
- `created_at`
- `updated_at`

---

## 7.2 FunctionVersion

### 定义

函数版本模型。

### 关键属性

- `version_id`
- `function_id`
- `source_code`
- `config`
- `published_at`
- `published_by`

---

## 7.3 Invocation

### 定义

函数调用记录。

### 关键属性

- `invocation_id`
- `function_id`
- `version_id`
- `caller_id`
- `input_payload`
- `status`
- `started_at`
- `finished_at`

---

## 7.4 InvocationResult

### 定义

函数调用结果。

### 关键属性

- `invocation_id`
- `stdout`
- `stderr`
- `exit_code`
- `time_used_ms`
- `memory_used_kb`
- `result_payload`

---

## 8. Runtime 执行模型

## 8.1 ExecutionTask

### 定义

统一执行任务抽象。

### 目标

为 `NexusRuntime` 提供跨业务域可复用的任务协议。

### 关键属性

- `task_id`
- `task_type`
- `source_domain`
- `source_entity_id`
- `runtime_kind`
- `payload`
- `resource_limits`
- `retry_policy`
- `created_at`

### 说明

`ExecutionTask` 是跨域任务协议，不等于数据库中的某个业务表。

---

## 8.2 ExecutionContext

### 定义

任务执行上下文。

### 包含内容

- 临时目录
- 环境变量
- 挂载资源
- 安全策略
- 网络权限

---

## 8.3 ResourceLimits

### 定义

统一资源限制模型。

### 关键属性

- `cpu_limit`
- `memory_limit_kb`
- `time_limit_ms`
- `process_limit`
- `network_policy`

---

## 8.4 ExecutionLog

### 定义

执行日志记录。

### 关键属性

- `task_id`
- `stage`
- `stream_type`
- `content`
- `timestamp`

---

## 8.5 ExecutionResult

### 定义

统一执行结果模型。

### 关键属性

- `task_id`
- `status`
- `exit_code`
- `time_used_ms`
- `memory_used_kb`
- `stdout`
- `stderr`
- `artifacts`

---

## 9. 平台统一状态模型

为了减少不同模块对状态词汇的混乱使用，建议定义一套统一状态语义。

## 9.1 通用异步状态

建议统一使用：

- `pending`
- `queued`
- `running`
- `success`
- `failed`
- `timeout`
- `cancelled`
- `dead_lettered`

### 适用场景

- Runtime 执行任务
- 云函数调用
- 发布任务
- OJ 提交流程中的异步阶段

---

## 9.2 OJ 结果状态

OJ 业务域可在其内部使用更细语义，例如：

- `accepted`
- `wrong_answer`
- `compile_error`
- `runtime_error`
- `time_limit_exceeded`
- `memory_limit_exceeded`
- `presentation_error`
- `internal_error`

### 原则

OJ 结果状态属于业务域细分，不应污染平台统一异步状态。

---

## 10. 统一事件模型设计目标

平台事件模型的存在是为了统一表达“系统发生了什么变化”。

事件模型的作用：

- 模块解耦
- 异步处理
- 实时推送
- 审计追踪
- 未来服务化

建议事件模型必须具备：

- 统一事件头
- 统一事件类型
- 统一发生时间
- 明确的来源域
- 明确的负载结构

---

## 11. 平台统一事件基础结构

每个事件建议都具有以下基础字段：

- `event_id`
- `event_type`
- `event_version`
- `domain`
- `aggregate_id`
- `occurred_at`
- `producer`
- `payload`
- `trace_id`

### 字段说明

- `event_id`：事件唯一标识
- `event_type`：事件类型，如 `submission.created`
- `event_version`：事件版本
- `domain`：来源业务域，如 `oj`、`content`
- `aggregate_id`：关联主实体 ID
- `occurred_at`：事件发生时间
- `producer`：事件生产方
- `payload`：事件负载
- `trace_id`：链路追踪 ID

---

## 12. 事件分类

建议将事件分为三类：

## 12.1 领域事件

表示某业务域内部发生的重要事实。

例如：

- `submission.created`
- `problem.updated`
- `document.published`
- `function.invoked`

## 12.2 执行事件

表示某个执行任务生命周期变化。

例如：

- `execution.queued`
- `execution.started`
- `execution.finished`
- `execution.failed`

## 12.3 通知事件

面向用户、前端或实时订阅的消息。

例如：

- `submission.status_updated`
- `cluster.node_updated`
- `publish.status_updated`

---

## 13. 推荐事件清单

## 13.1 OJ 事件

- `problem.created`
- `problem.updated`
- `submission.created`
- `submission.queued`
- `submission.judging`
- `submission.finished`
- `submission.failed`

## 13.2 内容事件

- `document.created`
- `document.updated`
- `document.published`
- `document.archived`
- `backlink.updated`

## 13.3 云函数事件

- `function.created`
- `function.version_published`
- `function.invocation_created`
- `function.invocation_started`
- `function.invocation_finished`
- `function.invocation_failed`

## 13.4 Runtime 事件

- `execution.task_created`
- `execution.task_queued`
- `execution.task_started`
- `execution.task_retrying`
- `execution.task_dead_lettered`
- `execution.task_finished`

## 13.5 集群与节点事件

- `runtime.node_registered`
- `runtime.node_heartbeat`
- `runtime.node_unhealthy`
- `runtime.node_removed`

---

## 14. 事件与实时推送的关系

平台事件不等于直接推给前端的消息。

推荐链路是：

1. 业务域或 runtime 产生内部事件
2. 业务状态被更新
3. `NexusGate` 或 `nexus-realtime` 将其转换为对外实时消息

原因：

- 内部事件更底层
- 前端消息更稳定
- 便于隐藏内部实现细节

例如：

- 内部事件：`execution.task_started`
- 对外 WS 消息：`submission.update`

---

## 15. 数据模型与事件模型之间的关系

可以这样理解：

- 数据模型回答：“系统里有什么”
- 事件模型回答：“系统里发生了什么”

### 举例

`Submission` 是数据模型。  
`submission.created` 是事件模型。

`Document` 是数据模型。  
`document.published` 是事件模型。

`ExecutionTask` 是数据模型。  
`execution.task_finished` 是事件模型。

---

## 16. 设计约束

为了保证长期可维护性，必须遵守以下约束：

### 16.1 业务域实体不直接当跨模块协议

例如：

- 不直接把数据库中的 `ProblemEntity` 发给 Runtime
- 不直接把数据库中的 `SubmissionEntity` 当 WebSocket 消息

### 16.2 事件负载不应塞入整表快照

事件应尽量：

- 明确
- 有边界
- 与用途匹配

不应动辄传整个领域对象的所有字段。

### 16.3 实时消息格式应独立于内部事件格式

前端协议应更稳定、可演进，不能被内部重构频繁打破。

### 16.4 Shared 中只放真正跨域共识模型

不应把所有业务类型都塞进共享模块。

---

## 17. 第一阶段建议优先统一的模型

如果平台要开始 Rust 化，第一批最应该先统一的是：

### 数据模型

- User
- ResourceIdentity
- Document
- Problem
- Submission
- ExecutionTask
- ExecutionResult
- PublishRecord

### 事件模型

- submission.created
- submission.finished
- execution.task_started
- execution.task_finished
- document.published
- function.invocation_created
- function.invocation_finished

---

## 18. 结论

`NexusCode` 的统一数据模型与事件模型设计，应坚持以下原则：

1. 平台公共模型与业务域模型分层
2. 内容域、OJ 域、Function 域各自保持独立领域语义
3. 执行系统使用统一任务模型
4. 事件模型作为跨模块解耦与未来服务化基础
5. 前端消息协议不直接暴露内部事件结构
6. 数据模型回答“对象是什么”，事件模型回答“对象发生了什么”

只要这套模型先定义清楚，后续无论你做：

- Rust 版 `NexusGate`
- Rust 版 `NexusRuntime`
- 博客系统
- 云函数系统
- Tauri 双链笔记发布链

都会比现在更容易保持边界稳定。
