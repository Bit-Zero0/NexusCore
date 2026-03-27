# Rust Workspace 详细模块划分文档

## 1. 文档目标

本文档用于定义 `NexusCode` 的 Rust Workspace 组织方式、模块职责、依赖方向、边界约束与后续拆分策略。

本文档解决的问题是：

- `NexusCode` 在 Rust 中应如何组织代码仓库
- 哪些模块属于平台公共能力，哪些属于业务域
- 模块之间应该如何依赖
- 为什么要先做模块化单体，而不是一开始就做微服务
- 后续如果平台继续扩张，应如何从 workspace 自然演进到多服务部署

本文档是架构设计文档，不包含具体代码实现。

---

## 2. 总体设计原则

### 2.1 模块化单体优先

`NexusCode` 第一阶段采用“模块化单体”路线：

- 一个 Rust workspace
- 多个 crate
- 一个主入口应用
- 一次部署即可运行核心能力

这样做的原因：

- 当前平台仍处于快速演化阶段
- 业务边界正在逐步明确
- 直接进入微服务会显著增加开发、调试、联调和运维成本

因此，第一阶段目标不是“服务数量多”，而是“边界清晰”。

### 2.2 边界先于部署

虽然第一阶段是单体部署，但在代码结构上必须先按未来服务边界来划分模块。

也就是说：

- 现在的“模块”，未来可以成长为“服务”
- 现在的函数调用，未来可以替换为 gRPC / HTTP / MQ 调用
- 现在的仓库边界，未来就是服务边界的基础

### 2.3 平台能力共享，业务能力隔离

需要共享的平台能力：

- API 接入
- 鉴权
- 实时推送
- 任务调度
- 执行内核
- 内容解析
- 存储访问规范

需要隔离的业务能力：

- OJ
- 博客与内容发布
- 云函数
- 笔记同步

### 2.4 强依赖方向控制

Rust workspace 中最重要的不是 crate 数量，而是依赖方向必须可控。

核心原则：

- 上层可以依赖下层
- 业务模块不能互相直接侵入内部实现
- 共享模块不能反向依赖业务模块
- gateway 只能调用业务模块公开的应用服务，不直接操作其内部 repository

---

## 3. 推荐 Workspace 结构

建议的 `NexusCode` workspace 结构如下：

```text
NexusCode/
  Cargo.toml
  rust-toolchain.toml
  crates/
    nexus-app/
    nexus-shared/
    nexus-config/
    nexus-gateway/
    nexus-auth/
    nexus-content/
    nexus-oj/
    nexus-runtime/
    nexus-function/
    nexus-realtime/
    nexus-storage/
    nexus-event/
    nexus-search/
    nexus-note-sync/
  docs/
  scripts/
  deployments/
```

---

## 4. 顶层模块说明

## 4.1 `nexus-app`

### 角色

主程序入口 crate。

### 职责

- 装配整个应用
- 加载配置
- 初始化数据库、Redis、MQ、对象存储等依赖
- 组装各业务模块
- 注册 HTTP / WebSocket 路由
- 启动后台任务和定时任务

### 不应承担

- 业务逻辑
- 判题逻辑
- 内容解析逻辑
- 复杂领域规则

### 说明

`nexus-app` 只是“装配器”，不应演化成“超大主程序”。

---

## 4.2 `nexus-shared`

### 角色

全平台共享基础类型与公共抽象。

### 职责

- 公共错误类型
- Result / Error 辅助类型
- 通用 ID 类型
- 时间、分页、审计等公共 DTO
- 通用 trait 定义
- 通用序列化结构

### 应包含

- `AppError`
- `PageRequest`
- `PageResponse`
- `UserId / ProblemId / SubmissionId / DocumentId`
- 通用事件元数据

### 不应包含

- 任何具体业务模块的复杂领域对象
- 直接依赖数据库驱动的实现细节

---

## 4.3 `nexus-config`

### 角色

统一配置管理模块。

### 职责

- 分层配置加载
- 环境变量覆盖
- dev / test / prod 配置切换
- 配置结构体统一定义

### 配置范围

- HTTP / WebSocket
- PostgreSQL
- Redis
- MQ
- 对象存储
- OJ 配置
- Runtime 配置
- Function 配置

---

## 4.4 `nexus-gateway`

### 角色

平台统一 API 入口层。

### 职责

- HTTP API
- WebSocket
- 请求鉴权入口
- 请求校验
- 对前端和客户端提供统一接口
- 聚合多个业务模块的返回结果

### 负责的能力

- 用户态 API
- 管理态 API
- 提交结果实时推送
- 发布任务状态推送
- 集群状态查询入口

### 不应承担

- 判题执行
- 云函数执行
- 文档 AST 变换
- 复杂领域持久化规则

### 未来拆分可能性

很高。`nexus-gateway` 未来可单独部署为平台统一入口服务。

---

## 4.5 `nexus-auth`

### 角色

统一身份与权限模块。

### 职责

- 用户管理
- 登录态
- Token / JWT
- 角色模型
- 资源访问控制
- 管理员与普通用户权限区分

### 领域对象

- User
- Session
- Role
- Permission
- AccessPolicy

### 依赖要求

- 不依赖 OJ、Content、Function 内部实现
- 其他模块可依赖它公开的认证/鉴权接口

---

## 4.6 `nexus-content`

### 角色

统一内容系统模块。

### 职责

- 文档管理
- Markdown 解析
- 平台扩展语法解析
- 双链与反向链接
- 发布状态管理
- 资源引用关系管理

### 核心对象

- Document
- Block
- Link
- Backlink
- AssetRef
- PublishRecord
- RenderProfile

### 未来用途

- 博客
- 知识库
- 题解
- 文档站
- 笔记发布内容源

### 特别要求

本模块未来应具备 AST 中间层，而不是只存字符串。

---

## 4.7 `nexus-oj`

### 角色

在线评测业务域模块。

### 职责

- 题目管理
- 测试用例管理
- 提交记录管理
- 题目导入
- 判题任务构造
- 判题模式配置管理
- 结果聚合与展示结构转换

### 支持模式

- ACM
- Functional
- EasyJudge
- Validator
- SPJ

### 特别说明

`EasyJudge` 可以在业务域内部完成轻量判定；其余需要执行型资源的任务通常会交给 `nexus-runtime`。

### 核心对象

- Problem
- ProblemStatement
- ProblemLimits
- Testcase
- Submission
- SubmissionResult
- JudgeConfig

---

## 4.8 `nexus-runtime`

### 角色

统一执行内核模块。

### 职责

- 编译
- 运行
- 沙箱
- 超时控制
- 内存限制
- CPU/进程控制
- 执行日志收集
- 状态回传
- 重试与失败处理基础能力

### 面向的任务类型

- OJ 判题任务
- 云函数执行任务
- 未来的脚本执行任务

### 不应承担

- 题目元数据编辑
- 文档内容管理
- 函数发布逻辑

### 设计定位

`nexus-runtime` 是“执行系统”，不是“业务系统”。

---

## 4.9 `nexus-function`

### 角色

云函数业务域模块。

### 职责

- 函数定义管理
- 版本管理
- 调用记录
- 触发策略
- 调用结果元数据

### 与 Runtime 的关系

本模块不直接执行函数，而是：

- 生成执行任务
- 提交给 `nexus-runtime`
- 消费执行结果

### 核心对象

- Function
- FunctionVersion
- Invocation
- Trigger
- InvocationLogRef

---

## 4.10 `nexus-realtime`

### 角色

统一实时通信抽象模块。

### 职责

- WebSocket 会话管理
- 订阅管理
- 心跳机制
- 事件编码与解码
- 重连恢复协议
- 多业务域实时事件统一协议

### 使用场景

- OJ 提交结果推送
- 集群状态推送
- 云函数执行流
- 发布任务状态更新

### 说明

若第一阶段实现较轻，也可先作为 `nexus-gateway` 内部模块，后续再抽 crate。

---

## 4.11 `nexus-storage`

### 角色

基础设施访问抽象层。

### 职责

- PostgreSQL 访问封装
- Redis 访问封装
- 对象存储客户端封装
- 连接池管理
- 通用 repository 辅助工具

### 注意

本模块只提供基础设施能力，不承载领域规则。

---

## 4.12 `nexus-event`

### 角色

统一事件总线抽象模块。

### 职责

- 定义事件发布/订阅接口
- 定义平台事件模型
- 单体阶段可提供内存实现
- 后续可切换 MQ / NATS / Kafka

### 事件示例

- SubmissionCreated
- SubmissionFinished
- ArticlePublished
- FunctionInvoked
- FunctionFinished

### 设计意义

它是未来从单体平滑演进到多服务的重要支点。

---

## 4.13 `nexus-search`

### 角色

搜索与索引模块。

### 职责

- 文档搜索
- 题目搜索
- 标签搜索
- 发布内容索引

### 说明

该模块可在后续阶段加入，第一阶段不必优先实现。

---

## 4.14 `nexus-note-sync`

### 角色

桌面笔记客户端同步模块。

### 职责

- 本地笔记与平台文档同步
- 发布记录同步
- 冲突检测
- 远程版本映射

### 对应客户端

- Tauri 双链笔记客户端

---

## 5. 依赖方向设计

推荐依赖关系如下：

```text
nexus-app
  -> nexus-config
  -> nexus-gateway
  -> nexus-auth
  -> nexus-content
  -> nexus-oj
  -> nexus-runtime
  -> nexus-function
  -> nexus-realtime
  -> nexus-storage
  -> nexus-event
  -> nexus-search
  -> nexus-note-sync
  -> nexus-shared

nexus-gateway
  -> nexus-auth
  -> nexus-content
  -> nexus-oj
  -> nexus-function
  -> nexus-realtime
  -> nexus-shared

nexus-oj
  -> nexus-event
  -> nexus-storage
  -> nexus-shared

nexus-function
  -> nexus-event
  -> nexus-storage
  -> nexus-shared

nexus-content
  -> nexus-storage
  -> nexus-shared

nexus-runtime
  -> nexus-event
  -> nexus-storage
  -> nexus-shared
```

---

## 6. 明确禁止的依赖关系

以下依赖方向应明确禁止：

### 6.1 业务模块互相直连内部实现

例如：

- `nexus-oj` 直接依赖 `nexus-content` 内部 repository
- `nexus-function` 直接依赖 `nexus-oj` 的表结构

### 6.2 Gateway 直接操作数据库

`nexus-gateway` 不应直接写 SQL 或直接操纵业务表。

### 6.3 Shared 依赖业务域

`nexus-shared` 必须保持底层公共模块定位，不能反向依赖 `nexus-oj` 或 `nexus-content`。

### 6.4 Runtime 侵入业务细节

`nexus-runtime` 不应该知道“题目页面如何显示”或“函数发布策略是什么”。

### 6.5 一个模块跨边界修改另一个模块的持久化模型

跨模块交互应通过：

- command
- query
- dto
- event

而不是直接拿对方内部持久化结构写入。

---

## 7. 模块内部推荐分层

每个业务 crate 推荐采用一致的内部结构：

```text
src/
  application/
  domain/
  infrastructure/
  interfaces/
  lib.rs
```

### 7.1 `application`

负责：

- 用例编排
- command / query handler
- 事务边界
- DTO 转换

### 7.2 `domain`

负责：

- 领域对象
- 领域规则
- 枚举
- 领域服务
- 领域事件

### 7.3 `infrastructure`

负责：
- 数据库实现
- Redis 实现
- MQ 实现
- 对象存储实现

### 7.4 `interfaces`

负责：

- HTTP handler
- WebSocket handler
- API request/response model

### 7.5 为什么需要统一结构

统一结构的好处：

- 降低维护成本
- 降低未来拆分难度
- 便于多人协作
- 有助于保持架构纪律

---

## 8. 数据库边界建议

第一阶段可以共用一个 PostgreSQL 实例，但建议在逻辑上分域。

推荐方式：

- `auth.*`
- `content.*`
- `oj.*`
- `function.*`

这样做的好处：

- 边界清晰
- 易于迁移
- 易于权限控制
- 易于后续拆分

不建议：

- 所有表混在一个默认 schema
- 表命名无业务边界

---

## 9. 单体阶段与未来服务化的映射关系

| 当前模块 | 未来是否适合拆分为独立服务 | 说明 |
|---|---|---|
| nexus-gateway | 是 | 统一入口，通常最先独立 |
| nexus-auth | 可能 | 用户系统成熟后可独立 |
| nexus-content | 是 | 内容与博客增长后可独立 |
| nexus-oj | 可能 | 若比赛/OJ业务复杂可独立 |
| nexus-runtime | 是 | 资源模型与 gateway 差异大，非常适合独立 |
| nexus-function | 是 | 云函数执行与管理后续可能独立 |
| nexus-realtime | 可能 | 高实时场景下可能独立 |
| nexus-event | 通常作为能力层 | 不一定独立为业务服务 |
| nexus-storage | 否 | 这是基础能力，不是服务 |

---

## 10. 第一阶段推荐最小落地集合

虽然 workspace 可以先规划完整，但第一阶段不需要全部实现。

建议最先落地的 crate：

- `nexus-app`
- `nexus-shared`
- `nexus-config`
- `nexus-gateway`
- `nexus-auth`
- `nexus-content`
- `nexus-oj`
- `nexus-runtime`
- `nexus-storage`
- `nexus-event`

暂缓实现：

- `nexus-function`
- `nexus-search`
- `nexus-note-sync`
- 独立的 `nexus-realtime`

原因：

- 先稳住平台核心入口
- 先打好 OJ 和内容系统基础
- 先定义统一执行能力

---

## 11. 为什么这套划分适合 NexusCode

`NexusCode` 的特殊性在于它不是单业务系统，而是：

- 有内容系统
- 有执行系统
- 有实时反馈
- 有发布系统
- 有客户端同步

如果没有清晰 workspace 边界，后面很容易出现：

- OJ 逻辑侵入博客系统
- 笔记发布逻辑侵入网关
- 云函数逻辑和 OJ 执行混成一团
- 内容模型和提交模型耦合

而本文档这套 workspace 划分，本质上是在提前预防这种架构退化。

---

## 12. 结论

`NexusCode` 的 Rust workspace 设计应遵循以下核心策略：

1. 先模块化单体，后服务化
2. 先边界清晰，再考虑部署拆分
3. 平台能力共享，业务域能力隔离
4. 执行内核单独抽象，不与业务域耦合
5. 内容系统作为平台一级能力，而不是博客附属模块
6. 统一入口 `nexus-gateway` 面向前端和客户端
7. 统一事件和统一配置模型应尽早建立

这套结构将为后续的：

- OJ Rust 重构
- 博客系统接入
- 云函数扩展
- Tauri 双链笔记发布

提供可持续演进的基础。
