# SPJ / Validator / Runtime 阶段性进展文档

## 1. 文档目的

本文档用于记录当前 `nexus-oj` 与 `nexus-runtime` 在 SPJ、Validator、Runtime Task、结果聚合、队列抽象、RabbitMQ 接入上的阶段性实现状态。

重点不是描述理想架构，而是描述当前仓库中的真实代码状态，便于后续继续推进时保持上下文一致。

---

## 2. 当前总体判断

当前重构方向是正确的，边界已经基本调整到位：

- `nexus-oj` 负责业务规则、判题任务生成、结果聚合、提交详情返回模型
- `nexus-runtime` 负责编译、执行、SPJ/Validator 比较、队列消费抽象
- `nexus-gateway` 负责入口装配、runtime 事件观察、Redis 实时推送

但当前仍处于“统一 runtime 内核已成型，真正的分布式 broker 消费尚未完成”的阶段。

一句话概括当前状态：

`领域边界已基本正确，消息语义已基本成型，真实 RabbitMQ 仅完成连接与 publish，尚未完成 reserve/ack/retry/reject 的 broker 化。`

---

## 3. 已完成的阶段性重构

## 3.1 OJ 配置校验补强

`nexus-oj` 已补齐 `JudgeConfig` 的关键合法性校验，能够在业务入口阶段拦截明显错误配置，包括：

- `judge_method=validator` 但缺少 `validator`
- `float_epsilon < 0`
- `spj language/source_code` 为空
- `judge_method` 与配置结构不匹配

这部分的意义是把“业务配置合法性”固定在 OJ 域内，而不是丢给 runtime 执行时报错。

---

## 3.2 Validator 与 SPJ 责任收敛到 Runtime

`nexus-runtime` 中已具备：

- Validator 比较逻辑
- SPJ 准备与执行逻辑
- 编译阶段与 case 阶段结果收集
- runtime 执行结果结构化输出

当前设计中，Validator/SPJ 不再被视为 OJ 内部杂糅逻辑，而是 runtime 执行链的一部分。

---

## 3.3 Runtime Event -> OJ 结果投影链路落位

结果投影主链路已经从 gateway 直接写 SQL 的方式，调整为：

`runtime event -> gateway observer -> oj_service.apply_runtime_event(...) -> repository 持久化`

这意味着：

- gateway 不再直接写业务表
- submission 状态映射回到 OJ 域内
- runtime 只负责发执行事件，不负责业务结果聚合

这一步是当前架构边界修正中非常关键的一步。

---

## 3.4 SubmissionDetail / SubmissionResult / SubmissionCaseResult 正式成型

提交详情返回模型已经不是“数据库原始字段透传”，而是形成了稳定的聚合结果对象：

- `SubmissionDetail`
- `SubmissionResult`
- `SubmissionCaseResult`

当前 OJ 查询层已经会从以下数据源聚合结果：

- `oj.submissions`
- `execution_summary`
- `oj.submission_case_results`
- `problem_testcases`

这意味着前端和上层 API 可以逐步摆脱对底层表结构和原始 JSON 的耦合。

---

## 3.5 RuntimeTask 元数据已为未来分池扩容铺路

`RuntimeTask` 目前已带有以下关键元数据：

- `source_domain`
- `source_entity_id`
- `queue`
- `lane`
- `retry_policy`

当前 OJ 生成任务时已按业务特征进行初步 lane 划分：

- 普通 ACM C++ / Rust: `fast`
- Python: `normal`
- Functional: `heavy`
- SPJ: `special`

这一步的价值在于：

即使当前还未真正接入 MQ 分池消费，协议模型已经开始为“按业务类型、按资源等级分池扩容”做准备。

---

## 3.6 Runtime 内存队列已具备接近真实 MQ 的语义

当前 `InMemoryRuntimeTaskQueue` 不再是简单 FIFO，而是已经具备：

- 按 `queue/lane` 分桶
- 跨 route 轮转
- `reserve`
- `ack`
- `retry`
- `reject`
- dead-letter 留存
- dead-letter replay
- queue stats

这使得 runtime 在尚未接入真实 MQ 时，已经把消费语义先固定下来了。

这是正确顺序：

先稳定语义，再接 broker。

---

## 3.7 RabbitMQ 适配层已进入“真实连接与 publish”阶段

当前 RabbitMQ 适配层已经不再是纯空壳。

已完成：

- RabbitMQ 配置校验
- 真实 AMQP 连接建立
- channel 建立
- `publisher confirms` 启用
- 按 `queue/lane` 动态声明 exchange / queue / retry queue / dead-letter queue
- `RuntimeTask` JSON 持久化 publish 到 RabbitMQ

同时已经去掉了已弃用的 `tokio-amqp` 扩展写法，改为 `lapin` 当前推荐的 executor/reactor 配置方式。

---

## 4. 当前真实链路

当前代码中的真实判题链路是：

`frontend -> gateway -> nexus-oj -> build RuntimeTask -> runtime schedule -> queue -> runtime worker -> runtime event -> gateway observer -> nexus-oj apply_runtime_event -> redis realtime push`

需要强调的几点：

1. 不是 `runtime -> OJ 判题`
这里 OJ 不是执行器，runtime 才是执行器。

2. 不是 gateway 直接写业务 SQL
gateway 只做 observer 和实时推送。

3. Redis 现在主要承担实时消息分发，不是结果权威存储
结果权威来源仍然是 OJ 域持久化结果。

---

## 5. 当前 RabbitMQ 接入状态

当前 RabbitMQ 接入还不是最终形态，而是 bootstrap 形态。

目前 `RabbitMqRuntimeTaskQueue` 的行为是：

1. 真实连接 RabbitMQ
2. 真实声明拓扑
3. 真实 publish 任务
4. publish 成功后，仍把任务写入本地内存队列

这样做的原因是：

当前系统还没有真正完成 RabbitMQ consumer 侧的 `reserve/ack/retry/reject` 映射，如果现在直接切掉本地队列，现有单进程 runtime 主链会断掉。

所以这一步是有意识的过渡方案，不是最终方案。

---

## 6. 当前已知风险与限制

## 6.1 RabbitMQ 仍未接管消费侧

这是当前最重要的未完成项。

目前 RabbitMQ 只完成了 publish，尚未完成：

- `reserve`
- `ack`
- `retry`
- `reject`
- lease/visibility
- dead-letter queue 回放的 broker 映射

这意味着目前还不能说 runtime 已经真正 broker 化。

---

## 6.2 当前存在“publish 到 RabbitMQ + 本地内存执行回退”的双轨过渡状态

这在当前阶段是合理的，但需要明确：

如果未来接入真正的 RabbitMQ consumer，而忘记移除本地 `inner.enqueue(...)` 回退逻辑，会产生重复执行风险。

所以在消费侧 broker 化完成后，这层回退必须被移除。

---

## 6.3 Runtime 集群能力仍未完成

虽然协议层已支持 `queue/lane/source_domain`，但当前还没有真正做到：

- MQ 分队列消费
- 不同 worker group 独立部署
- 按业务池扩容
- OJ / Function 资源隔离

因此当前仍不能把 runtime 视作“可水平扩容的统一执行集群”，只能视作“方向已定、协议已准备、实现正在推进中的执行内核”。

---

## 7. 阶段性 code review 结论

本轮 review 已确认并处理以下问题：

- 去掉 RabbitMQ 接入中的已弃用 `tokio-amqp` 扩展写法
- 补上 `publisher confirms`
- 收敛 runtime 中几处会影响严格 `clippy` 的实现问题
- 收敛 OJ 中一处低风险校验代码结构问题

当前状态下：

- `cargo check -q` 通过
- `cargo test -q` 全仓通过
- `cargo clippy -q -p nexus-runtime -- -D warnings` 通过
- `cargo clippy -q -p nexus-gateway -- -D warnings` 通过

---

## 8. 下一阶段建议

下一阶段建议按下面顺序推进：

1. 完成 RabbitMQ 消费侧适配
目标：
- `reserve/ack/retry/reject` 真正映射到 broker
- 去掉本地内存回退执行

2. 设计并实现 retry queue / dead-letter queue 的真实 broker 行为
目标：
- broker 层面可重试
- broker 层面死信留存
- dead-letter replay 可重投

3. 开始按 `queue/lane` 拆 runtime worker group
目标：
- `oj_judge.fast`
- `oj_judge.heavy`
- `oj_judge.special`
- `function.invoke.*`

4. 给 RabbitMQ 路径补集成测试
目标：
- 不只是单元测试通过
- 而是真实验证 publish / consume / ack / retry / dead-letter 行为

---

## 9. 当前结论

当前这轮重构已经完成了“边界纠偏”和“消息语义搭骨架”这两个最重要的阶段。

也就是说，我们已经不再处在“旧 OJ/Judger 逻辑混合堆叠”的状态，而是开始进入：

`OJ 负责业务，Runtime 负责执行，Broker 负责消息，Gateway 负责入口与实时事件`

但仍需保持清醒：

现在还不是最终完成态，真正决定这套架构能否站稳的下一步，是把 RabbitMQ 的消费链真正接通。

