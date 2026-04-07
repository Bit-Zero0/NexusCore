#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <broker:rabbitmq|nats|redis_streams> [role:embedded|runtime-worker]" >&2
  exit 1
fi

broker="$1"
role="${2:-runtime-worker}"
port="${PORT:-19080}"
startup_timeout="${STARTUP_TIMEOUT_SECONDS:-45}"
recovery_timeout="${RECOVERY_TIMEOUT_SECONDS:-60}"
log_dir="${LOG_DIR:-/tmp/nexus-broker-drills}"
log_file="${log_dir}/${broker}-${role}.log"

mkdir -p "$log_dir"

case "$broker" in
  rabbitmq)
    env_file="env/dev.compose.monitoring.rabbitmq.env"
    compose_service="rabbitmq"
    health_mode="docker-health"
    health_target="nexus-rabbitmq"
    ;;
  nats)
    env_file="env/dev.compose.monitoring.nats.env"
    compose_service="nats"
    health_mode="http"
    health_target="http://127.0.0.1:8222/varz"
    ;;
  redis_streams)
    env_file="env/dev.compose.monitoring.redis_streams.env"
    compose_service="redis"
    health_mode="docker-health"
    health_target="nexus-redis"
    ;;
  *)
    echo "unsupported broker: $broker" >&2
    exit 1
    ;;
esac

case "$role" in
  embedded|runtime-worker) ;;
  *)
    echo "unsupported role: $role" >&2
    exit 1
    ;;
esac

set -a
source "$env_file"
set +a

export NEXUS_PROCESS_ROLE="$role"
export NEXUS_BIND_ADDR="127.0.0.1:${port}"
export NEXUS_OJ_REPOSITORY="${OJ_REPOSITORY_OVERRIDE:-memory}"

case "$broker" in
  rabbitmq)
    export NEXUS_RUNTIME_RABBITMQ_EXCHANGE="drill.${broker}.${role}"
    export NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX="drill.${broker}.${role}"
    ;;
  nats)
    stream_suffix="$(echo "${role}" | tr '[:lower:]-.' '[:upper:]__')"
    export NEXUS_RUNTIME_NATS_STREAM="DRILL_${stream_suffix}"
    export NEXUS_RUNTIME_NATS_SUBJECT_PREFIX="drill.${broker}.${role}"
    export NEXUS_RUNTIME_NATS_CONSUMER_PREFIX="drill.${broker}.${role}"
    ;;
  redis_streams)
    export NEXUS_RUNTIME_REDIS_STREAMS_PREFIX="drill.${broker}.${role}"
    export NEXUS_RUNTIME_REDIS_STREAMS_GROUP_PREFIX="drill.${broker}.${role}"
    export NEXUS_RUNTIME_REDIS_STREAMS_CONSUMER_PREFIX="drill.${broker}.${role}"
    ;;
esac

cleanup() {
  if [[ -n "${app_pid:-}" ]] && kill -0 "$app_pid" >/dev/null 2>&1; then
    kill "$app_pid" >/dev/null 2>&1 || true
    wait "$app_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

cargo run -p nexus-app >"$log_file" 2>&1 &
app_pid=$!

app_url="http://127.0.0.1:${port}"
timeout "${startup_timeout}" bash -c "until curl -fsS ${app_url}/healthz >/dev/null && curl -fsS ${app_url}/metrics >/dev/null; do sleep 2; done"
if [[ "$role" == "runtime-worker" ]]; then
  timeout "${startup_timeout}" bash -c "until curl -fsS ${app_url}/api/v1/runtime/broker >/dev/null; do sleep 2; done"
fi

echo "App is healthy before broker restart: ${broker}/${role}"
docker compose -f docker-compose.dev.yml restart "$compose_service" >/dev/null

if [[ "$health_mode" == "docker-health" ]]; then
  timeout "${recovery_timeout}" bash -c "until [ \"\$(docker inspect -f '{{.State.Health.Status}}' ${health_target})\" = 'healthy' ]; do sleep 2; done"
else
  timeout "${recovery_timeout}" bash -c "until curl -fsS ${health_target} >/dev/null; do sleep 2; done"
fi

timeout "${recovery_timeout}" bash -c "until curl -fsS ${app_url}/healthz >/dev/null && curl -fsS ${app_url}/metrics >/dev/null; do sleep 2; done"
if [[ "$role" == "runtime-worker" ]]; then
  timeout "${recovery_timeout}" bash -c "until curl -fsS ${app_url}/api/v1/runtime/broker >/dev/null; do sleep 2; done"
fi

echo "Broker recovery verified for ${broker}/${role}"
echo "Application log: ${log_file}"
