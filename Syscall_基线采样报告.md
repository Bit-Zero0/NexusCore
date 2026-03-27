# Runtime Syscall 基线采样报告

更新时间：2026-03-27  
采样机器：当前本地开发机（WSL/Linux）  
采样方式：`strace -f -qq`  
采样脚本：[collect_runtime_syscall_baseline.sh](/home/fmy/Nexus_OJ/NexusCode/scripts/collect_runtime_syscall_baseline.sh)

## 结论

这次已经把三条最关键的运行期基线跑出来了：

- `cpp native`
- `rust native`
- `wasmtime + wasm32-wasip1`

结论有三点最重要：

1. `NexusJudger` 的旧 `BASE_SYSCALLS` 可以继续作为参考，但不能直接照抄。  
原因是当前机器的 `strace` 里已经能看到明显的发行版/工具链差异，比如 `fstat`、`statx`，而旧项目里主要是 `newfstat`、`newfstatat`。

2. `strace` 原始结果里的 `execve` 不能直接等同于“运行期必须允许 execve”。  
原始采样会把“程序启动时那一次 execve”也记进去。对于 OJ 运行期 seccomp，真正要回答的是：
`用户程序在已经启动后，是否还需要再次 exec 子进程？`
这个问题和原始 `strace` 集合不是一回事。

现在脚本已经会同时输出三层视图：

- `raw`: 原始 syscall 集合
- `steady`: 去掉 bootstrap-only 项（当前先去掉 `execve`）
- `normalized`: 把 `fstat/newfstat/newfstatat/statx` 这类别名折叠成语义组

3. `wasmtime` 的 syscall 面明显比 `cpp/rust native` 大。  
所以 `wasm` 和 `nsjail_wasm` 的 seccomp 绝不能直接复用 `cpp_default`，必须单独维护一条更宽但受控的 runtime allowlist。

## 当前机器实测基线

### C++ native

`raw`

```text
access arch_prctl brk close execve exit_group fstat futex getrandom lseek mmap mprotect munmap openat pread64 prlimit64 read rseq set_robust_list set_tid_address write
```

`steady`

```text
access arch_prctl brk close exit_group fstat futex getrandom lseek mmap mprotect munmap openat pread64 prlimit64 read rseq set_robust_list set_tid_address write
```

`normalized`

```text
access arch_prctl brk close exit_group file_stat futex getrandom lseek mmap mprotect munmap openat pread64 prlimit64 read rseq set_robust_list set_tid_address write
```

### Rust native

`raw`

```text
access arch_prctl brk close execve exit_group fstat getrandom gettid mmap mprotect munmap openat poll pread64 prlimit64 read rseq rt_sigaction sched_getaffinity set_robust_list set_tid_address sigaltstack write
```

`steady`

```text
access arch_prctl brk close exit_group fstat getrandom gettid mmap mprotect munmap openat poll pread64 prlimit64 read rseq rt_sigaction sched_getaffinity set_robust_list set_tid_address sigaltstack write
```

`normalized`

```text
access arch_prctl brk close exit_group file_stat getrandom gettid mmap mprotect munmap openat poll pread64 prlimit64 read rseq scheduler_affinity set_robust_list set_tid_address signal_runtime write
```

### Wasmtime

`raw`

```text
access arch_prctl brk clock_nanosleep clone3 close epoll_create1 epoll_ctl epoll_wait eventfd2 execve exit exit_group fcntl fstat futex getdents64 getpid getpriority getrandom gettid ioctl lseek madvise memfd_create mkdir mmap mprotect munmap openat poll prctl pread64 prlimit64 read readlink rename rseq rt_sigaction rt_sigprocmask sched_getaffinity sched_yield set_robust_list set_tid_address setpriority sigaltstack socketpair statx unlink write
```

`steady`

```text
access arch_prctl brk clock_nanosleep clone3 close epoll_create1 epoll_ctl epoll_wait eventfd2 exit exit_group fcntl fstat futex getdents64 getpid getpriority getrandom gettid ioctl lseek madvise memfd_create mkdir mmap mprotect munmap openat poll prctl pread64 prlimit64 read readlink rename rseq rt_sigaction rt_sigprocmask sched_getaffinity sched_yield set_robust_list set_tid_address setpriority sigaltstack socketpair statx unlink write
```

`normalized`

```text
access arch_prctl brk clock_nanosleep close epoll_create1 epoll_ctl epoll_wait eventfd2 exit exit_group fcntl file_stat futex getdents64 getpid getpriority getrandom gettid ioctl lseek madvise memfd_create mkdir mmap mprotect munmap openat poll prctl pread64 prlimit64 process_clone read readlink rename rseq scheduler_affinity set_robust_list set_tid_address setpriority signal_runtime socketpair unlink write
```

## 与 NexusJudger 的对照

旧项目 [seccomp_rules.cpp](/home/fmy/Nexus_OJ/NexusJudger/src/execution/seccomp_rules.cpp) 当前启用的 `BASE_SYSCALLS` 主要是：

```text
execve access arch_prctl brk clock_gettime close exit_group faccessat futex getrandom lseek mmap mprotect munmap pread64 prlimit64 read readlink readv rseq set_robust_list set_tid_address write writev open openat newfstat ioctl wait4 unlink clone3 rt_sigaction rt_sigprocmask newfstatat
```

对照下来：

- 旧规则里有：`newfstat`、`newfstatat`
- 当前机器实测里更常直接看到：`fstat`
- `wasmtime` 额外出现：`epoll_*`、`eventfd2`、`memfd_create`、`socketpair`、`statx`、`madvise`、`sched_yield`、`prctl`
- Rust native 相对 C++ native 额外出现：`poll`、`gettid`、`sigaltstack`、`sched_getaffinity`

## 别名与归一化建议

为了避免你之前踩过的“同语义不同名字”问题，建议不要只维护一份平铺 syscall 名单，而是同时维护一层“归一化语义组”。

### 推荐的归一化组

- `file_stat`
  - `fstat`
  - `newfstat`
  - `newfstatat`
  - `statx`

- `process_clone`
  - `clone`
  - `clone3`
  - `vfork`

- `signal_runtime`
  - `rt_sigaction`
  - `rt_sigprocmask`
  - `sigaltstack`
  - `rt_sigreturn`

- `scheduler_affinity`
  - `sched_getaffinity`
  - `sched_yield`

### 工程建议

真正落 seccomp 时：

1. 先维护一份“归一化能力组”
2. 再按当前发行版/内核/工具链展开成具体 syscall 名称
3. 测试时同时校验：
   - 原始 syscall 是否被覆盖
   - 归一化能力组是否完整

这样后面从 Debian/Ubuntu 切到别的发行版时，不需要整份 allowlist 重写。

## 对当前 NexusCode 的建议

### `cpp/rust native`

可以基于当前实测集合继续收窄，但要注意：

- `execve` 不应该因为原始 `strace` 里出现就直接保留给运行期
- 应区分：
  - `bootstrap syscalls`
  - `steady-state runtime syscalls`

### `wasmtime`

单独建一条 runtime allowlist，不要复用 `cpp_default`。  
否则要么误杀 `wasmtime`，要么把 native 路线放得太宽。

### 编译期

编译期 seccomp 仍建议单独维护，不和运行期合并。  
`rustc` / `clang++` / `g++` 的 syscall 面都比运行期宽，尤其会涉及：

- `execve`
- 更多文件元数据调用
- 进程/线程创建

## 当前建议的下一步

1. 已完成：在 `nexus-runtime` 里新增“归一化 syscall 组”层，不再只维护平铺字符串列表。
2. 已完成：为 `cpp_native`、`rust_native`、`wasmtime` 各自维护独立的运行期 allowlist。
3. 已完成：新增 [compare_runtime_seccomp_profiles.sh](/home/fmy/Nexus_OJ/NexusCode/scripts/compare_runtime_seccomp_profiles.sh)，可以把 `strace` 基线和当前 seccomp profile 展开做自动 diff。
4. 下一步：把 `execve bypass` 两条当前被 `ignored` 的测试，等运行期 allowlist 校准完成后再转正。

## 对比工作流

现在可以直接用下面这条命令跑一轮“采样 vs 当前配置”对比：

```bash
bash /home/fmy/Nexus_OJ/NexusCode/scripts/compare_runtime_seccomp_profiles.sh
```

它会做三件事：

1. 重新采样 `cpp / rust / wasmtime` 的运行期 syscall 基线
2. 通过 `nexus-runtime` 的调试导出拿到当前 `cpp_native_default / rust_native_default / wasm_default` profile 展开结果
3. 输出两层 diff：
   - `steady_missing / steady_extra`
   - `norm_missing / norm_extra`

其中：

- `steady_*` 用来看当前 profile 和原始运行期 syscall 的实际差异
- `norm_*` 用来看归一化语义组层面的差异，避免被 `fstat/newfstatat/statx` 这类别名噪声误导
