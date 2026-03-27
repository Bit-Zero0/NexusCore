# OJ 性能测试与瓶颈分析

## 1. 测试时间
- 日期：2026-03-27

## 2. 测试环境
- 宿主机：当前开发机
- Docker 容器：
  - RabbitMQ：`rabbitmq:3-management`，端口 `5672/15672`
  - Redis：`redis:latest`，端口 `6379`
  - Postgres：`postgres:16-alpine`，端口 `55432`
- 后端进程：
  - `127.0.0.1:8082`：`embedded + memory queue`
  - `127.0.0.1:8083`：`embedded + rabbitmq queue`
  - `127.0.0.1:8084`：修正 `time_used_ms` 口径后的验证实例

## 3. 测试对象
- 题目：`two-sum`
- 判题模式：`acm`
- 语言：
  - `cpp`
  - `python`
- 测试用例：
  - 输入：`1 2`
  - 标准输出：`3`

## 4. 测试脚本
- 基准脚本：[`scripts/bench_oj_submission.mjs`](/home/fmy/Nexus_OJ/NexusCode/scripts/bench_oj_submission.mjs)
- 脚本能力：
  - 创建提交
  - 轮询直到终态
  - 拉取 runtime task snapshot
  - 输出 `e2eMs / resultMs / compileMs / caseMs`

## 5. 基准结果

### 5.1 C++，memory queue，5 次顺序提交
- 平均端到端耗时：`2956 ms`
- 平均 `SubmissionResult.time_used_ms`：`2838 ms`
- 平均 compile 耗时：`2797 ms`
- 平均 testcase 执行耗时：`41 ms`
- 最小端到端耗时：`2740 ms`
- 最大端到端耗时：`3055 ms`

### 5.2 C++，RabbitMQ，5 次顺序提交
- 平均端到端耗时：`2956 ms`
- 平均 `SubmissionResult.time_used_ms`：`2829 ms`
- 平均 compile 耗时：`2792 ms`
- 平均 testcase 执行耗时：`37 ms`
- 最小端到端耗时：`2754 ms`
- 最大端到端耗时：`3117 ms`

### 5.3 Python，memory queue，5 次顺序提交
- 平均端到端耗时：`169 ms`
- 平均 `SubmissionResult.time_used_ms`：`63 ms`
- 平均 compile 耗时：`0 ms`
- 平均 testcase 执行耗时：`63 ms`

## 6. 结论

### 6.1 你看到的 `3129 ms` 主要不是“程序运行慢”
- 旧实现里，`SubmissionResult.time_used_ms` 统计口径是：
  - `compile`
  - `judge_compile`
  - `cases`
- 对 C++ 这类需要编译的语言，这会把编译时间直接展示成“耗时”。
- 所以最简单的两数相加也会显示成 `3s` 左右。

### 6.2 真正的主瓶颈是 C++ 编译
- 在 C++ ACM 单测下，compile 阶段约占总耗时的 `94%` 到 `98%`。
- testcase 真正运行只在 `37 ms` 到 `49 ms` 这一量级。
- 换成 Python 后，整体端到端直接下降到 `169 ms`，进一步说明瓶颈不在 testcase 执行。

### 6.3 RabbitMQ 不是当前瓶颈
- `memory queue` 与 `rabbitmq queue` 的平均端到端时间几乎完全一致。
- 当前这条链路下，MQ 开销相对 compile 阶段可以忽略。

### 6.4 OJ 当前展示耗时的语义有误导性
- OJ 用户通常关心的是：
  - 程序执行时间
  - 而不是编译时间
- 因此 `SubmissionResult.time_used_ms` 更合理的口径应为 testcase 执行时间之和。

## 7. 本轮已修复
- 已将 OJ 结果中的 `time_used_ms` 改为只统计 testcase 执行时间。
- 修复后 live 验证结果：
  - `displayTimeMs = 43 ms`
  - `compileMs = 3117 ms`
  - `caseMs = 43 ms`

这说明：
- 编译仍然慢，但不再错误展示为“程序运行耗时”

## 8. 当前已识别瓶颈

### 8.1 编译阶段过重
- C++ 每次提交都完整重新编译。
- 当前编译命令为：
  - `g++ -std=c++20 -O2 -pipe -o main main.cpp`
- 这是当前 OJ 链路最大的单点耗时来源。

### 8.2 运行阶段仍有固定沙箱成本
- 即使 testcase 极简单，单 case 仍然在几十毫秒量级。
- 当前主要来自：
  - `nsjail` 进程启动
  - mount namespace
  - user namespace
  - cgroup 限制设置
  - `/bin/sh -lc` 包装

### 8.3 指标模型还不够细
- 当前用户态结果模型没有单独暴露：
  - `compile_time_ms`
  - `judge_compile_time_ms`
  - `run_time_ms`
- 导致前端难以同时展示“编译耗时”和“运行耗时”。

## 9. 优化建议

### 9.1 高优先级
- 保持 `time_used_ms` 只表示 testcase 运行时间。
- 新增独立字段：
  - `compile_time_ms`
  - `judge_compile_time_ms`
  - `total_judge_time_ms`

### 9.2 中优先级
- 减少 C++ 编译成本：
  - 评估 `-O2` 是否对 OJ 默认档位过重
  - 评估更轻的默认编译参数
  - 后续考虑对象缓存或编译缓存

### 9.3 中优先级
- 降低运行阶段固定开销：
  - 评估去掉 `/bin/sh -lc` 包装，直接 exec 目标命令
  - 评估精简每次启动的沙箱准备成本
  - 评估 worker 侧更轻的执行包装

### 9.4 架构层
- `standard / validator` 命名已经开始收口。
- 当前正式语义为：
  - 默认判题就是 `validator`
  - `spj` 单独表示特殊判题
- 旧的 `standard` 仅作为兼容别名接收，不再作为正式返回值。

## 10. 总结
- 当前 OJ 模块“慢”的核心不是 RabbitMQ，也不是 testcase 执行本身。
- 核心瓶颈是编译阶段，尤其是 C++。
- 你看到的 `3129 ms` 主要来自旧的耗时口径错误，已经修正为只展示 testcase 执行时间。
- 如果下一轮继续优化，最值得做的是：
  - 细化时间指标模型
  - 优化编译链
  - 继续细化 validator 默认策略与显式策略的展示
