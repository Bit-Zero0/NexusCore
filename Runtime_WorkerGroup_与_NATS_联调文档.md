# Runtime Worker Group 与 NATS 联调文档

本文档用于说明如何在本地通过 Docker 启动 NATS JetStream，并将 `nexus-app` 的 runtime broker 切换到 NATS。

## 1. 目标

当前 NATS 适配已接入：

- runtime task publish
- runtime worker reserve
- ack
- retry delay
- reject
- dead-letter replay

当前 dead-letter 记录仍保存在应用内存中，适合作为第一阶段联调版本。

## 2. Docker 启动 NATS

最简单的方式是直接使用 Docker 启动带 JetStream 的 `nats-server`：

```bash
docker run -d \
  --name nexus-nats \
  -p 4222:4222 \
  -p 8222:8222 \
  nats:2.10 \
  -js
```

启动后：

- NATS 地址：`nats://127.0.0.1:4222`
- HTTP 监控地址：`http://127.0.0.1:8222`

如果本地镜像代理对 `nats:2.10` 拉取异常，可尝试：

```bash
docker run -d \
  --name nexus-nats \
  -p 4222:4222 \
  -p 8222:8222 \
  nats:latest \
  -js
```

## 3. Runtime / Gateway 环境变量

切换到 NATS 时，设置：

```bash
export NEXUS_RUNTIME_BROKER_BACKEND=nats
export NEXUS_RUNTIME_NATS_URL="nats://127.0.0.1:4222"
export NEXUS_RUNTIME_NATS_STREAM="NEXUS_RUNTIME"
export NEXUS_RUNTIME_NATS_SUBJECT_PREFIX="nexus.runtime"
export NEXUS_RUNTIME_NATS_CONSUMER_PREFIX="nexus-runtime"
```

如果你还在使用旧变量名，当前代码仍兼容：

```bash
export NEXUS_RUNTIME_QUEUE_BACKEND=nats
```

但后续建议统一迁移到：

`NEXUS_RUNTIME_BROKER_BACKEND`

## 4. 启动应用

例如以嵌入模式启动：

```bash
export NEXUS_PROCESS_ROLE=embedded
cargo run -p nexus-app
```

或以 runtime worker 模式启动：

```bash
export NEXUS_PROCESS_ROLE=runtime-worker
cargo run -p nexus-app
```

## 5. NATS 集成测试

NATS 集成测试使用：

`NEXUS_NATS_TEST_URL`

例如：

```bash
export NEXUS_NATS_TEST_URL="nats://127.0.0.1:4222"
cargo test -p nexus-runtime nats -- --nocapture
```

如果没有设置 `NEXUS_NATS_TEST_URL`，测试会直接返回，适合本地未启动 NATS 的开发场景。

## 6. 当前限制

当前版本的 NATS 适配有两个明确限制：

- dead-letter 记录仍由应用内存维护，进程重启后不会保留
- 统计信息以 consumer pending 与应用侧 leased/dead-letter 视图为主，不是完整的 broker 运维视图

这两个点都适合放到下一阶段继续增强。
