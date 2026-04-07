# Job 平台 / Runtime / Broker 职责边界文档

## 1. 文档目的

本文档用于明确三层职责边界：

- Job 平台
- Runtime 执行层
- Broker 传输层

这份文档的目标不是重复实现细节，而是回答：

- 每一层负责什么
- 每一层不负责什么
- 哪些信息应该停留在 Job 层
- 哪些信息可以下沉到 Runtime 或 Broker

---

## 2. 分层总览

当前统一异步执行体系分为三层：

1. Job 平台：业务抽象层
2. Runtime：执行协调层
3. Broker：消息传输层

一句话概括：

`Job 平台负责业务语义，Runtime 负责执行语义，Broker 负责传输语义。`

---

## 3. Job 平台职责

Job 平台负责：

- 定义统一 Job 抽象
- 接收业务模块提交的 `JobDefinition`
- 进行平台级校验
- 解析/校验 handler 注册关系
- 记录 Job 定义、提交历史、失败历史
- 提供 Job 查询/管理接口
- 为控制面提供 Job 级状态与历史视图

当前相关实现集中在：

- [crates/nexus-jobs/src/model](/home/fmy/NexusCore/crates/nexus-jobs/src/model)
- [crates/nexus-jobs/src/api](/home/fmy/NexusCore/crates/nexus-jobs/src/api)
- [crates/nexus-jobs/src/handlers](/home/fmy/NexusCore/crates/nexus-jobs/src/handlers)

Job 平台不负责：

- 具体消息如何发送到 RabbitMQ/NATS/Redis
- worker 如何 reserve/ack/retry
- dead-letter 的底层持久化策略
- broker reclaim / reconnect 细节
- 容器/沙箱执行细节

---

## 4. Runtime 职责

Runtime 负责：

- 接收已经平台化后的可执行任务
- 将任务交给 worker 执行
- 维护运行生命周期
- 处理执行过程中的 retry / reject / dead-letter / replay
- 产生 runtime event 与 runtime management 视图
- 对接 broker adapter

当前相关实现集中在：

- [crates/nexus-runtime/src/executor.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/executor.rs)
- [crates/nexus-runtime/src/router.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/router.rs)
- [crates/nexus-runtime/src/broker](/home/fmy/NexusCore/crates/nexus-runtime/src/broker)

Runtime 不负责：

- 定义业务模块的 Job 抽象
- 决定某个业务模块如何建模 payload
- 解释博客/OJ/云函数的业务结果
- 暴露业务模块专属的控制面

也就是说：

Runtime 不应重新承载业务语义。

---

## 5. Broker 职责

Broker 负责：

- 消息传输
- 队列/流的路由与消费
- ack / retry / reject
- dead-letter 的底层存储
- replay 的底层读取与恢复
- reconnect / reclaim / recovery
- queue stats

当前相关实现集中在：

- [crates/nexus-runtime/src/broker/mod.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/mod.rs)
- [crates/nexus-runtime/src/broker/rabbitmq.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/rabbitmq.rs)
- [crates/nexus-runtime/src/broker/nats.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/nats.rs)
- [crates/nexus-runtime/src/broker/redis.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/redis.rs)

Broker 不负责：

- 解释 Job 是 OJ、云函数还是博客任务
- 校验业务 payload
- 维护业务模块级的结果模型
- 暴露业务模块级管理接口

也就是说：

Broker 永远不应上浮成业务层接口。

---

## 6. 哪些信息必须停留在 Job 层

以下信息必须停留在 Job 平台，而不应泄漏到 Broker：

- `JobType`
- `JobNamespace`
- 业务 payload 语义
- 业务来源与 origin
- handler descriptor / capabilities
- Job 查询与管理视图
- 模块级失败语义
- 提交阶段失败与 handler rejection

原因是这些信息属于：

- 业务模块如何接入
- 控制面如何理解任务
- 平台如何保持与具体 Broker 解耦

---

## 7. 哪些信息可以下沉到 Runtime

以下信息可以由 Job 平台映射后下沉到 Runtime：

- route
- retry policy
- timeout policy
- execution contract
- dispatch plan

原因是这些已经属于“任务如何被执行”的语义，而不是“业务如何理解任务”的语义。

---

## 8. 哪些信息可以继续下沉到 Broker

以下信息可以由 Runtime 继续映射后下沉到 Broker：

- queue / lane
- lease / ack 语义
- retry delay
- dead-letter 读写
- replay / reclaim / recovery

这些信息属于传输与消费实现细节，不应回流污染 Job 抽象。

---

## 9. 当前边界映射关系

当前三层的关系可以概括为：

### 9.1 Job -> Runtime

由 `JobHandler` 和 dispatch plan 完成映射。

相关实现：

- [crates/nexus-jobs/src/handlers/contract.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/handlers/contract.rs)
- [crates/nexus-jobs/src/runtime/submitter.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/runtime/submitter.rs)
- [crates/nexus-jobs/src/runtime/mapper.rs](/home/fmy/NexusCore/crates/nexus-jobs/src/runtime/mapper.rs)

### 9.2 Runtime -> Broker

由 runtime queue 与 broker adapter 完成映射。

相关实现：

- [crates/nexus-runtime/src/executor.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/executor.rs)
- [crates/nexus-runtime/src/broker/mod.rs](/home/fmy/NexusCore/crates/nexus-runtime/src/broker/mod.rs)

### 9.3 Broker -> 管理视图

由 runtime management API 暴露 broker 运行状态；
由 Job API 暴露 Job 级视图；
控制面聚合两者，而不是让其中一层替代另一层。

---

## 10. 当前明确禁止的反模式

为了保持边界稳定，以下做法明确不允许：

1. 把 Job 平台重新设计成另一套 Broker
2. 让业务模块继续直接调用 broker adapter
3. 让 Runtime 再次承载业务模块特有语义
4. 在 Broker 层存放博客/OJ/云函数的业务解释逻辑
5. 让控制面直接依赖某一种 Broker 的专有对象模型

---

## 11. 控制面如何使用三层信息

控制面应按下面方式使用三层信息：

- Job 平台：看任务是谁、属于哪个模块、现在是什么状态、最近失败是什么
- Runtime：看执行层是否健康、worker 是否正常、任务执行链是否稳定
- Broker：看消息层是否堵塞、是否处于恢复窗口、是否有 dead-letter/reclaim 异常

控制面不应：

- 直接把 broker queue 视图当作业务任务视图
- 直接把 runtime event 当成最终业务语义

---

## 12. 当前阶段结论

当前仓库已经具备比较清晰的三层边界：

- Job 平台不再直接暴露 broker 细节
- Runtime 不再直接承载上层业务建模
- Broker 已收敛到统一适配语义

这意味着后续继续扩展：

- 云函数模块
- 博客异步任务
- 统一 Job 控制面

都可以建立在这套稳定分层之上，而不需要重新拆分职责。

---

## 13. 一句话结论

这三层的最佳实践边界应始终保持为：

`Job 平台定义业务任务，Runtime 负责执行协调，Broker 负责消息传输。`

只要这个边界不被打破，后续模块扩展和平台演进就不会重新退回“业务语义和消息语义混杂”的状态。
