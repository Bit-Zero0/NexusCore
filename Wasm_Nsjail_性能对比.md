# Wasm 与 nsjail 判题性能对比

## 测试时间

- 2026-03-27

## 测试环境

- 后端：本机 `nexus-app`，地址 `http://127.0.0.1:8080`
- 队列：RabbitMQ
- 运行时：
  - `wasmtime 43.0.0`
  - Rust target `wasm32-wasip1`
  - `clang-17`
  - `lld-17`
  - `wasi-libc`
  - `libc++-17-dev-wasm32`
  - `libc++abi-17-dev-wasm32`
  - `libclang-rt-17-dev-wasm32`

## 基准方法

- 脚本：[`scripts/bench_wasm_vs_nsjail.mjs`](/home/fmy/Nexus_OJ/NexusCode/scripts/bench_wasm_vs_nsjail.mjs)
- 对比维度：
  - `cpp + nsjail`
  - `cpp + wasm`
  - `rust + nsjail`
  - `rust + wasm`
- 每组运行 5 次
- 题型：单测试点 ACM，两数相加
- 统计字段：
  - `e2e_ms`：创建提交到拿到终态结果的端到端耗时
  - `compile_ms`：编译阶段耗时
  - `run_ms`：真正运行时间
  - `memory_kb`：结果页展示内存

## 原始汇总

| 组别 | runs | avg_e2e_ms | avg_compile_ms | avg_run_ms | avg_memory_kb | min_e2e_ms | max_e2e_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| cpp:nsjail | 5 | 424 | 217 | 63 | 7142 | 253 | 943 |
| cpp:wasm | 5 | 330 | 135 | 65 | 7142 | 302 | 366 |
| rust:nsjail | 5 | 524 | 343 | 53 | 7270 | 462 | 639 |
| rust:wasm | 5 | 509 | 332 | 54 | 7194 | 466 | 523 |

## 去掉首轮冷启动后的观察

首轮通常会带来编译器冷启动、缓存预热、磁盘页面缓存建立等影响。把每组第 1 次去掉后，结论更稳定：

- `cpp + nsjail`
  - warm `avg_e2e_ms ≈ 295`
  - warm `avg_compile_ms ≈ 121`
  - warm `avg_run_ms ≈ 44`
- `cpp + wasm`
  - warm `avg_e2e_ms ≈ 322`
  - warm `avg_compile_ms ≈ 131`
  - warm `avg_run_ms ≈ 68`
- `rust + nsjail`
  - warm `avg_e2e_ms ≈ 495`
  - warm `avg_compile_ms ≈ 317`
  - warm `avg_run_ms ≈ 51`
- `rust + wasm`
  - warm `avg_e2e_ms ≈ 507`
  - warm `avg_compile_ms ≈ 334`
  - warm `avg_run_ms ≈ 52`

## 结论

### 1. 当前这台机器上，Wasm 没有表现出稳定的性能优势

- `rust` 路线基本接近持平，但 `wasm` 没有明显更快
- `cpp` 路线在去掉冷启动后，`nsjail` 反而略快

### 2. 当前收益主要不是性能，而是执行形态

- `wasm` 的价值更偏：
  - 更统一的执行模型
  - 更适合作为 Rust 云函数运行容器
  - 更容易做能力收敛和后续平台复用
- 不是“天然更快”

### 3. 目前 `rust + wasm` 已经具备稳定最小可用链路

- smoke test 已通过
- 基准结果也稳定
- 可以继续作为 OJ 与云函数的候选执行后端推进

### 4. 目前 `cpp + wasm` 还是“最小可用”，还不是“完整兼容”

这轮基准使用的 C++ 代码是 `cstdio/scanf/printf` 路线。  
使用 `iostream` 的最小试编曾触发 `__cxa_allocate_exception / __cxa_throw` 相关链接问题，说明：

- 现在的 `cpp + wasm` 已经能跑最小 ACM 样例
- 但离“完整覆盖常见 OJ C++ 写法”还有距离
- 在正式对出题人开放前，应该补更多兼容性验证

## 建议

### 短期

- 保持 `cpp/rust` 的 `nsjail` 主链不变
- 将 `rust + wasm` 视为优先推进对象
- `cpp + wasm` 先标记为实验性能力

### 中期

- 补更多 Wasm 基准：
  - 多测试点
  - 更大输入
  - 纯计算题
  - I/O 密集题
- 给 `cpp + wasm` 补标准库/异常支持验证

### 工程结论

- `Wasm` 值得继续做
- 但当前不应该因为这轮结果就把 `nsjail` 替掉
- 更合理的定位是：
  - `nsjail` 继续做稳定主链
  - `wasm` 作为新增执行后端逐步成熟
