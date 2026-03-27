# Linux Syscall Flavor 适配文档

更新时间：2026-03-27

## 目标

这份文档记录 `nexus-runtime` 当前的 Linux syscall 兼容档位设计，用于说明：

- 为什么不能只维护一份平铺 syscall allowlist
- 当前已经扩展到什么程度
- 各 Linux family 与 architecture 组合之间的主要差异
- 现在的边界和后续该怎么继续推进

## 当前支持的 Linux family

配置项：`NEXUS_RUNTIME_SYSCALL_FLAVOR`

可选值：

- `auto`
- `generic`
- `debian_ubuntu`
- `arch`
- `rhel_like`

当前实现位置：

- 运行时配置：[lib.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-config/src/lib.rs)
- seccomp 展开与自动检测：[executor.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-runtime/src/executor.rs)
- 调试导出工具：[seccomp_profiles.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-runtime/src/bin/seccomp_profiles.rs)

`auto` 模式会根据 `/etc/os-release` 解析当前 Linux 家族：

- Debian / Ubuntu -> `debian_ubuntu`
- Arch / Manjaro -> `arch`
- Fedora / RHEL / Rocky / Alma / CentOS -> `rhel_like`
- 其他情况 -> `generic`

## 当前支持的 architecture

配置项：`NEXUS_RUNTIME_SYSCALL_ARCH`

可选值：

- `auto`
- `x86_64`
- `aarch64`
- `other`

`auto` 模式会根据当前运行架构自动解析：

- `x86_64` -> `x86_64`
- `aarch64` -> `aarch64`
- 其他情况 -> `other`

当前 seccomp 生成逻辑已经不是只看 `flavor`，而是同时看：

- Linux family
- architecture

也就是说，现在的真实判断维度是：

- `debian_ubuntu + x86_64`
- `debian_ubuntu + aarch64`
- `rhel_like + x86_64`
- `rhel_like + aarch64`
- 以及其他保守组合

## 当前已经 flavor 化的 syscall 组

### 1. `runtime_core`

这是当前所有运行期 profile 都会经过的一层基础能力组。

当前展开策略：

- `generic / debian_ubuntu / arch`
  - 保持当前主环境行为
  - 继续带：
    - `clock_gettime`
    - `faccessat`
    - `readlink`
    - `readv`
    - `writev`

- `rhel_like`
  - 第一阶段先保守收紧：
    - 不主动带 `readv`
    - 不主动带 `writev`
  - 其他核心项仍保留：
    - `futex`
    - `mmap/mprotect/munmap`
    - `pread64`
    - `prlimit64`
    - `read/write`
    - `rseq`

说明：

- 这不是说 `rhel_like` 一定不需要 `readv/writev`
- 只是当前先让 `runtime_core` 也具备 flavor 化结构
- 是否回补这两个 syscall，后续要以 RHEL-like 实机采样为准

### 2. `file_stat_compat`

这是当前最重要、最容易跨发行版漂移的一组。

归一化语义：

- `fstat`
- `newfstat`
- `newfstatat`
- `statx`

当前展开：

- `generic`
  - `newfstat`
  - `newfstatat`

- `debian_ubuntu`
  - `fstat`
  - `newfstat`
  - `newfstatat`
  - `statx`

- `arch`
  - `fstat`
  - `newfstatat`
  - `statx`

- `rhel_like`
  - `newfstat`
  - `newfstatat`
  - `statx`

### 3. `thread_compat`

归一化语义：

- `clone`
- `clone3`
- `vfork`

当前展开：

- `generic`
  - `clone`
  - `clone3`

- `debian_ubuntu`
  - `clone`
  - `clone3`

- `arch`
  - `clone`
  - `clone3`

- `rhel_like`
  - `clone3`

说明：

- 这组还处于第一阶段 flavor 化
- `rhel_like` 目前刻意更保守
- 后续是否补 `clone`，应基于实机采样，而不是凭经验直接放开

### 4. `signal_compat`

归一化语义：

- `rt_sigaction`
- `rt_sigprocmask`
- `sigaltstack`
- `rt_sigreturn`

当前展开：

- `generic`
  - `rt_sigaction`
  - `rt_sigprocmask`
  - `sigaltstack`

- `debian_ubuntu`
  - `rt_sigaction`
  - `rt_sigprocmask`
  - `sigaltstack`
  - `rt_sigreturn`

- `arch`
  - `rt_sigaction`
  - `rt_sigprocmask`
  - `sigaltstack`

- `rhel_like`
  - `rt_sigaction`
  - `rt_sigprocmask`

说明：

- `rt_sigreturn` 当前只在 `debian_ubuntu` 先显式展开
- `rhel_like` 仍然保守，不主动带 `sigaltstack`

### 5. `file_open_compat`

归一化语义：

- `open`
- `openat`
- `readlink`
- `readlinkat`
- `unlink`

当前展开：

- `generic`
  - `open`
  - `openat`
  - `readlinkat`
  - `unlink`

- `debian_ubuntu`
  - `open`
  - `openat`
  - `readlink`
  - `readlinkat`
  - `unlink`

- `arch`
  - `open`
  - `openat`
  - `readlinkat`
  - `unlink`

- `rhel_like`
  - `openat`
  - `readlinkat`
  - `unlink`

说明：

- `rhel_like` 先不主动放 `open`
- `debian_ubuntu` 先显式带 `readlink`

### 6. `wasmtime_runtime_extras`

这是当前最需要单独对待的一组，因为 `wasmtime` 的 syscall 面明显大于 native `cpp/rust`。

当前展开策略：

- `generic / debian_ubuntu / arch`
  - 保持相对完整
  - 包含 `memfd_create`
  - 包含 `sched_yield`

- `rhel_like`
  - 先保守裁掉：
    - `memfd_create`
    - `sched_yield`
  - 其他核心项仍保留：
    - `epoll_*`
    - `eventfd2`
    - `prctl`
    - `socketpair`
    - `madvise`
    - `getdents64`

说明：

- 这组目前还是“第一阶段 flavor 化”
- `rhel_like` 的裁剪是保守策略，不代表这些 syscall 永远不需要
- 是否回补，必须以实机采样为准

架构补充：

- 当 `arch = aarch64` 时，会额外显式带：
  - `epoll_pwait`
  - `getpid`
  - `membarrier`
  - `mkdirat`
  - `ppoll`
  - `renameat`

- 这批规则来自当前已经采到的 `Oracle Ubuntu ARM64` 样本
- 现阶段先按 `aarch64` 收口，不直接把它们并入所有 `debian_ubuntu`

### 7. `process_lifecycle_compat`

归一化语义：

- `wait4`
- `waitid`

当前展开：

- `generic`
  - `wait4`

- `debian_ubuntu`
  - `wait4`
  - `waitid`

- `arch`
  - `wait4`

- `rhel_like`
  - `wait4`
  - `waitid`

说明：

- `waitid` 目前先只在 `debian_ubuntu / rhel_like` 显式展开
- `generic / arch` 仍然保守，只保留 `wait4`

### 8. `rust_runtime_extras`

归一化语义：

- `gettid`
- `poll`
- `sched_getaffinity`

当前展开：

- `generic`
  - `gettid`
  - `poll`
  - `sched_getaffinity`

- `debian_ubuntu`
  - `gettid`
  - `poll`
  - `sched_getaffinity`

- `arch`
  - `gettid`
  - `poll`
  - `sched_getaffinity`

- `rhel_like`
  - `gettid`
  - `poll`

说明：

- `rhel_like` 目前不主动带 `sched_getaffinity`
- 这仍然是保守策略，后续是否回补看实机采样

架构补充：

- 当 `arch = aarch64` 时，当前会额外显式带：
  - `ppoll`

- 这条也是基于当前 `Oracle Ubuntu ARM64` 实机采样先补上的

### 9. `cpp_runtime_extras`

归一化语义：

- `ioctl`

当前展开：

- `generic`
  - `ioctl`

- `debian_ubuntu`
  - `ioctl`

- `arch`
  - `ioctl`

- `rhel_like`
  - 当前为空

说明：

- 这组当前只对 `rhel_like` 做保守收紧
- 不是说 `rhel_like` 一定不需要 `ioctl`
- 只是当前先不预设放开，等实机采样再决定是否回补

### 10. `python_runtime_extras`

归一化语义：

- `dup`
- `dup2`
- `getcwd`
- `getdents64`
- `getegid`
- `geteuid`
- `getgid`
- `gettid`
- `getuid`
- `pipe2`
- `readlink`
- `sysinfo`

当前展开：

- `generic`
  - `dup`
  - `dup2`
  - `getcwd`
  - `getdents64`
  - `getegid`
  - `geteuid`
  - `getgid`
  - `gettid`
  - `getuid`
  - `pipe2`
  - `sysinfo`

- `debian_ubuntu`
  - `dup`
  - `dup2`
  - `getcwd`
  - `getdents64`
  - `getegid`
  - `geteuid`
  - `getgid`
  - `gettid`
  - `getuid`
  - `pipe2`
  - `readlink`
  - `sysinfo`

- `arch`
  - `dup`
  - `dup2`
  - `getcwd`
  - `getdents64`
  - `getegid`
  - `geteuid`
  - `getgid`
  - `gettid`
  - `getuid`
  - `pipe2`
  - `sysinfo`

- `rhel_like`
  - `dup`
  - `dup2`
  - `getcwd`
  - `getdents64`
  - `getegid`
  - `geteuid`
  - `getgid`
  - `gettid`
  - `getuid`
  - `pipe2`

说明：

- `debian_ubuntu` 先显式带 `readlink`
- `rhel_like` 先保守不主动带 `sysinfo`
- 这仍然是第一阶段 flavor 化，后续以实机采样为准

### 11. `compiler_extras`

这是编译期 profile 中除 `execve` 之外的补充 syscall 组。

当前展开策略：

- `generic / debian_ubuntu / arch`
  - 保持当前主环境行为
  - 包含：
    - `dup`
    - `dup2`
    - `getcwd`
    - `getegid`
    - `geteuid`
    - `getgid`
    - `gettid`
    - `getuid`
    - `ioctl`
    - `pipe2`
    - `sysinfo`

- `rhel_like`
  - 第一阶段先保守不主动带 `sysinfo`
  - 其他项保持不变

说明：

- 编译期 syscall 面本来就比运行期宽
- 这组目前只是做了最小 flavor 化，还没有按不同编译器做更细拆分

### 12. `compiler_exec`

这是编译期 profile 里专门负责“拉起编译器进程”的 syscall 组。

当前展开策略：

- `generic`
  - `execve`

- `debian_ubuntu`
  - `execve`
  - `execveat`

- `arch`
  - `execve`
  - `execveat`

- `rhel_like`
  - `execve`

说明：

- 这组目前也是第一阶段 flavor 化
- `debian_ubuntu / arch` 先显式带 `execveat`
- `generic / rhel_like` 仍然保守只保留 `execve`
- 是否在 `rhel_like` 回补 `execveat`，后续要以实机采样和真实编译链路验证为准

## 当前还没有 flavor 化的组

当前主要 syscall 组已经全部进入 flavor 机制。

这并不表示适配已经完成，而是表示：

- 主组已经都有了 flavor-aware 结构
- 但仍然缺少足够多的跨发行版实机采样去校准这些保守策略

## 调试与对比

### 导出当前 seccomp profile

```bash
cargo run -q --manifest-path ./Cargo.toml \
  -p nexus-runtime --bin seccomp_profiles -- --flavor=debian_ubuntu --arch=aarch64 cpp_native_default
```

### 对比采样与当前 profile

```bash
bash scripts/compare_runtime_seccomp_profiles.sh /tmp/nexus-seccomp-compare debian_ubuntu
```

也可以直接批量跑全部 flavor：

```bash
bash scripts/compare_runtime_seccomp_profiles.sh /tmp/nexus-seccomp-compare --all-flavors
```

输出会包含：

- `steady_missing`
- `steady_extra`
- `norm_missing`
- `norm_extra`

以及当前用于展开 seccomp 的：

- `flavor`
- `arch`

## 当前覆盖范围

可以把目前状态理解为：

- 已经从“单 host 展开”升级成“可配置 Linux family + architecture 展开”
- 已经有：
  - 配置入口
  - 自动检测
  - 调试导出
  - 采样对比
  - flavor 级测试

但还没有做到：

- Debian / Ubuntu / Arch / RHEL 全部实机验证完成
- 形成最终稳定的多发行版 allowlist 基线

## 下一步建议

1. 在至少一台 Debian/Ubuntu x86_64 和一台 RHEL-like x86_64 机器上各跑一次采样。
2. 把当前这台 `Oracle Ubuntu ARM64` 样本单独归档，不要和通用 Ubuntu x86_64 混用。
3. 基于真实 diff 决定是否继续细化：
   - `wasmtime_runtime_extras` 的更细子组
   - `compiler_extras` 的编译器级子组
   - `compiler_exec` 在 `rhel_like` 是否应回补 `execveat`
   - `runtime_core` 里 `readv/writev` 是否应在 `rhel_like` 回补
   - `aarch64` 是否还需要额外的 `wasmtime` syscall
4. 给 `compare_runtime_seccomp_profiles.sh` 增加主机元信息归档能力。
5. 最后再决定默认 `auto` 是否足够稳，还是需要在生产环境明确指定 `flavor + arch`。

## 实机采样计划

多发行版实机采样的执行计划已经单独整理在：

- [Linux_多发行版_Syscall_实机采样计划.md](/home/fmy/Nexus_OJ/NexusCore/Linux_多发行版_Syscall_实机采样计划.md)
