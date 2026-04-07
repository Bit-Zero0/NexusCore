# Job 平台业务模块统一映射说明

## 1. 文档目的

本文档用于说明统一 Job 平台如何承接三类上层业务模块：

- OJ 模块
- 云函数模块
- 博客模块

重点不是展开完整产品方案，而是明确：

- 谁提交 Job
- 谁消费 Job
- 谁查看结果
- 谁处理失败
- 三类模块如何映射到同一套 Job 接入方式

---

## 2. 统一接入模式

三类业务模块统一遵循下面这套模式：

1. 业务模块构建 `JobDefinition`
2. 业务模块通过 `JobPlatformService` 提交 Job
3. Job 平台完成校验、记录定义、记录提交事件
4. 已注册的 `JobHandler` 负责把 Job 转成可执行 dispatch 计划
5. Runtime 执行层消费任务并产生运行事件
6. Job 查询/管理接口聚合定义、状态、历史、失败信息供控制面查询

这意味着业务模块不直接操作：

- RabbitMQ / NATS / Redis Streams
- runtime queue 细节
- dead-letter / replay 细节
- broker recovery / reclaim 细节

---

## 3. OJ 模块映射

## 3.1 谁提交

当前由 OJ 应用服务负责构建并提交 Job。

相关实现：

- [crates/nexus-oj/src/application.rs](/home/fmy/NexusCore/crates/nexus-oj/src/application.rs)
- [crates/nexus-oj/src/lib.rs](/home/fmy/NexusCore/crates/nexus-oj/src/lib.rs)

当前 OJ 的提交流程是：

1. 创建 submission
2. 基于 submission 构建 `JobDefinition`
3. 调用 `JobPlatformService.submit(...)`

---

## 3.2 谁消费

当前由 OJ 对应的 `JobHandler` 负责消费准备，最终映射为 runtime judge task。

相关实现：

- [crates/nexus-jobs/src/domains/oj.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/domains/oj.rs)

当前 OJ handler 的职责是：

- 校验 payload 是否为 `OjJudge`
- 声明自身能力与执行契约
- 将 Job 映射成 `RuntimeTask`

---

## 3.3 谁查看结果

结果查看分为两层：

- OJ 业务结果：submission/judge outcome 仍由 OJ 领域对象负责展示
- Job 平台状态：通过 Job 查询接口与历史接口查看

相关接口：

- `GET /api/v1/jobs`
- `GET /api/v1/jobs/:job_id`
- `GET /api/v1/jobs/:job_id/history`
- `GET /api/v1/jobs/management/overview`

---

## 3.4 谁负责失败处理

失败处理按层分工：

- 业务层关心 submission 最终是否成功判题
- Job 平台关心 handler 是否拒绝 dispatch
- Runtime/Broker 层关心 retry、dead-letter、replay、reclaim

OJ 模块不直接处理：

- broker 级 dead-letter
- broker 级 replay
- broker recovery

这些由底层平台统一负责。

---

## 3.5 当前阶段结论

OJ 已经是第一批正式接入对象，且已经通过现有实现验证了：

- Job 定义构建
- Job 平台提交
- handler 注册与执行契约
- runtime 映射
- 查询与历史链路

所以 OJ 已满足“第一批落地验证对象”的要求。

---

## 4. 云函数模块映射

## 4.1 谁提交

未来由云函数网关或函数触发器负责提交 Job。

典型来源包括：

- HTTP 调用函数
- 定时触发
- 事件触发
- 队列触发

这些触发器都会统一构建 `JobDefinition`，而不是直接调 runtime queue。

---

## 4.2 谁消费

未来由云函数专用 `JobHandler` 消费准备。

云函数 handler 的职责应包括：

- 校验函数版本与入口元数据
- 装配执行上下文
- 选择 route / lane
- 映射为可执行 dispatch 计划

第一阶段建议仍然通过 `RuntimeTask` 承载执行。

后续如果需要专用执行器，也应继续保留统一 Job 接口，而不是绕开 Job 平台。

---

## 4.3 谁查看结果

结果查看应分成两层：

- 函数调用结果、执行日志、业务返回值：由云函数模块自身负责
- Job 生命周期、调度历史、失败重试状态：由 Job 平台负责

换句话说：

- Job 平台负责“有没有被接收、有没有进入执行、有没有失败重试”
- 云函数模块负责“函数执行结果是什么”

---

## 4.4 谁负责失败处理

失败处理边界建议如下：

- handler 拒绝 dispatch：Job 平台可见并返回结构化错误
- runtime 执行失败：走 retry/dead-letter/replay
- 函数业务错误：作为函数执行结果的一部分由云函数模块解释

---

## 4.5 当前阶段结论

云函数模块当前还未正式实现，但已经可以映射到现有 Job 接入模式：

- 提交端：触发器 -> `JobPlatformService`
- 消费端：`CloudFunctionJobHandler`
- 执行端：Runtime
- 观测端：Job 查询/管理接口 + runtime 管理视图

因此它是 OJ 之后最适合的第二批验证对象。

---

## 5. 博客模块映射

## 5.1 谁提交

未来由博客模块在需要异步化的场景中提交 Job。

典型场景包括：

- 异步索引
- 内容审核
- 通知推送
- 静态化生成
- 媒体处理

博客模块不会把“文章发布”本身全部建立在 Job 平台上，而是只把异步任务接到 Job 平台。

---

## 5.2 谁消费

未来由博客专用 `JobHandler` 消费准备。

不同任务类型可分别注册：

- `blog:index_post`
- `blog:moderate_content`
- `blog:send_notification`
- `blog:build_static_page`

这些 handler 统一遵循当前平台的：

- descriptor
- capabilities
- execution context
- dispatch result

---

## 5.3 谁查看结果

结果查看同样分两层：

- 博客业务结果：例如文章是否已审核、索引是否完成、静态页是否生成
- Job 平台结果：任务是否已提交、是否失败、是否 dead-letter、是否 replay

---

## 5.4 谁负责失败处理

博客模块自身负责解释业务结果；
Job 平台负责统一处理任务级失败与调度级失败。

这意味着博客模块不需要自己维护另一套：

- retry 队列
- dead-letter 队列
- replay 操作

---

## 5.5 当前阶段结论

博客模块当前尚未实现，但它对 Job 平台的复用方式已经清楚：

- 不是“博客整体跑在 Job 上”
- 而是“博客的异步能力统一接入 Job 平台”

因此它更适合作为第三批复用对象，而不是第一批验证对象。

---

## 6. 三类模块的统一映射结论

三类模块虽然业务目标不同，但都能映射到同一套平台边界：

- 提交者：业务模块应用服务/触发器
- 提交入口：`JobPlatformService`
- 消费准备：模块专属 `JobHandler`
- 执行层：Runtime
- 传输层：Broker
- 查询/管理：Job API + Runtime 管理视图

统一点在于：

- 不直接碰 Broker
- 不直接写 runtime queue 语义
- 不单独设计自己的失败重试体系

---

## 7. 第一批落地对象结论

第一批落地验证对象明确为：

`OJ`

原因是：

- 当前已经有最成熟的 runtime/broker 接入基础
- 当前已经真实接入 Job 平台
- 当前最容易验证提交、消费、查询、历史、失败链路是否自洽

推荐顺序为：

1. OJ
2. 云函数
3. 博客

---

## 8. 一句话结论

OJ、云函数、博客三类模块虽然业务形态不同，但都可以通过：

`业务模块构建 JobDefinition -> JobPlatformService 提交 -> 模块专属 JobHandler -> Runtime 执行 -> Job API/管理视图查询`

这同一条接入链路统一接入 Job 平台。
