# Linux 多发行版 Syscall 实机采样计划

更新时间：2026-03-27

## 目标

这份计划用于把当前 `nexus-runtime` 的 Linux syscall flavor 机制，从“结构已具备”推进到“有实机数据校准”。

当前我们已经有：

- `NEXUS_RUNTIME_SYSCALL_FLAVOR`
- `NEXUS_RUNTIME_SYSCALL_ARCH`
- `auto / generic / debian_ubuntu / arch / rhel_like`
- `auto / x86_64 / aarch64 / other`
- `seccomp_profiles` 调试导出工具
- `collect_runtime_syscall_baseline.sh`
- `compare_runtime_seccomp_profiles.sh`

现在缺的是：

- 至少一台 `debian_ubuntu`
- 至少一台 `rhel_like`
- 最好再补一台 `arch`

这些机器上的真实采样结果。

## 采样目标矩阵

建议最少覆盖这几类主机：

1. Debian/Ubuntu x86_64
   例如：Ubuntu 22.04 / 24.04，Debian 12

2. RHEL-like x86_64
   例如：Rocky Linux 9 / AlmaLinux 9 / CentOS Stream 9

3. Arch x86_64
   例如：Arch Linux / Manjaro

4. 已有样本
   `Oracle Ubuntu ARM64`

每类主机至少采样一次。当前 ARM 样本已经有一份，但它应单独看待。

## 采样前准备

目标主机需要满足：

- 能运行 `cargo`
- 能运行 `strace`
- `ptrace` 没被额外限制
- 本机已经能构建 `NexusCode`

建议先确认：

```bash
strace -V
cargo --version
rustc --version
```

## 单 flavor 采样步骤

在目标主机上进入仓库目录：

```bash
cd /root/NexusCore
```

先跑单 flavor 对比：

```bash
bash scripts/compare_runtime_seccomp_profiles.sh \
  /tmp/nexus-seccomp-compare \
  auto
```

如果要显式指定 family 与 architecture：

```bash
bash scripts/compare_runtime_seccomp_profiles.sh \
  /tmp/nexus-seccomp-compare \
  debian_ubuntu
```

然后单独导出 profile：

```bash
cargo run -q --manifest-path ./Cargo.toml \
  -p nexus-runtime --bin seccomp_profiles -- \
  --flavor=debian_ubuntu --arch=aarch64 --json > /tmp/nexus-seccomp-compare/seccomp_profiles_debian_ubuntu_aarch64.json
```

输出重点看：

- `steady_missing`
- `steady_extra`
- `norm_missing`
- `norm_extra`

## 批量 flavor 对比步骤

当前脚本已经支持批量模式：

```bash
bash scripts/compare_runtime_seccomp_profiles.sh \
  /tmp/nexus-seccomp-compare \
  --all-flavors
```

它会依次输出：

- `generic`
- `debian_ubuntu`
- `arch`
- `rhel_like`

这适合在同一台主机上快速观察“当前 host 基线”与不同 flavor 设定之间的差异。

## 每台主机需要收集的材料

每跑完一台主机，建议至少保存：

1. `/etc/os-release`
2. `uname -a`
3. `uname -m`
4. `compare_runtime_seccomp_profiles.sh` 的完整输出
5. `/tmp/nexus-seccomp-compare/baseline_report.txt`
6. `/tmp/nexus-seccomp-compare/seccomp_profiles_*.json`

建议按这样的目录归档：

```text
artifacts/
  syscall-sampling/
    ubuntu-24.04/
    oracle-ubuntu-arm64/
    rocky-9/
    arch-latest/
```

## 优先校准的组

拿到实机数据后，优先看这些保守策略是否需要回补：

1. `runtime_core`
   当前 `rhel_like` 先不主动带：
   - `readv`
   - `writev`

2. `compiler_exec`
   当前 `rhel_like` 先不主动带：
   - `execveat`

3. `compiler_extras`
   当前 `rhel_like` 先不主动带：
   - `sysinfo`

4. `cpp_runtime_extras`
   当前 `rhel_like` 先不主动带：
   - `ioctl`

5. `rust_runtime_extras`
   当前 `rhel_like` 先不主动带：
   - `sched_getaffinity`

6. `wasmtime_runtime_extras`
  当前 `rhel_like` 先不主动带：
  - `memfd_create`
  - `sched_yield`

7. `aarch64` 专项
   当前已经基于样本先补：
   - `rust_runtime_extras -> ppoll`
   - `wasmtime_runtime_extras -> epoll_pwait / getpid / membarrier / mkdirat / ppoll / renameat`

   后面要确认这些是否是：
   - 通用 `aarch64` 现象
   - 还是 `Oracle Ubuntu ARM64` 的特定现象

## 判定原则

如果某个 syscall 在某类主机上持续出现：

- 出现在 `steady_missing`
- 且属于正常运行必须项

那就应考虑把它加回对应 flavor。

如果某个 syscall 只出现在：

- `steady_extra`
- `norm_extra`
- 且没有实机必要性

那就可以继续保持当前保守策略。

## 当前阶段结论

现在我们不是缺工具，而是缺样本。

也就是说：

- flavor 机制已经具备
- 批量对比脚本已经具备
- 调试导出已经具备

下一步真正值钱的，是把 Debian/Ubuntu、RHEL-like、Arch 三类主机的实机数据跑出来，然后再收紧或回补当前这些保守项。
