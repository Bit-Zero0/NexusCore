#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

brokers_csv="${BROKERS:-rabbitmq,nats,redis_streams}"
roles_csv="${ROLES:-embedded,gateway,runtime-worker}"
IFS=',' read -r -a brokers <<<"$brokers_csv"
IFS=',' read -r -a roles <<<"$roles_csv"

base_port="${BASE_PORT:-18080}"
startup_timeout="${STARTUP_TIMEOUT_SECONDS:-35}"
poll_interval="${POLL_INTERVAL_SECONDS:-1}"
oj_repository="${OJ_REPOSITORY_OVERRIDE:-memory}"
log_dir="${LOG_DIR:-/tmp/nexus-runtime-matrix}"

mkdir -p "$log_dir"

cleanup_pid() {
  local pid="$1"
  if kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
}

check_endpoint() {
  local role="$1"
  local url_base="$2"
  curl -fsS "${url_base}/healthz" >/dev/null
  case "$role" in
    embedded)
      curl -fsS "${url_base}/metrics" >/dev/null
      curl -fsS "${url_base}/api/v1/system/health" >/dev/null
      ;;
    gateway)
      curl -fsS "${url_base}/metrics" >/dev/null
      ;;
    runtime-worker)
      curl -fsS "${url_base}/metrics" >/dev/null
      curl -fsS "${url_base}/api/v1/runtime/broker" >/dev/null
      ;;
  esac
}

run_case() {
  local broker="$1"
  local role="$2"
  local port="$3"
  local env_file="env/dev.compose.${broker}.env"
  local log_file="${log_dir}/${broker}-${role}.log"
  local url_base="http://127.0.0.1:${port}"
  local namespace="smoke.${broker}.${role}.${port}"

  echo "==> smoke ${broker} / ${role}"

  set -a
  source "$env_file"
  set +a

  export NEXUS_PROCESS_ROLE="$role"
  export NEXUS_BIND_ADDR="127.0.0.1:${port}"
  export NEXUS_OJ_REPOSITORY="$oj_repository"

  case "$broker" in
    rabbitmq)
      export NEXUS_RUNTIME_RABBITMQ_EXCHANGE="${namespace}"
      export NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX="${namespace}"
      ;;
    nats)
      export NEXUS_RUNTIME_NATS_STREAM="$(echo "${namespace}" | tr '[:lower:].-' '[:upper:]__')"
      export NEXUS_RUNTIME_NATS_SUBJECT_PREFIX="${namespace}"
      export NEXUS_RUNTIME_NATS_CONSUMER_PREFIX="${namespace}"
      ;;
    redis_streams)
      export NEXUS_RUNTIME_REDIS_STREAMS_PREFIX="${namespace}"
      export NEXUS_RUNTIME_REDIS_STREAMS_GROUP_PREFIX="${namespace}"
      export NEXUS_RUNTIME_REDIS_STREAMS_CONSUMER_PREFIX="${namespace}"
      ;;
  esac

  cargo run -p nexus-app >"$log_file" 2>&1 &
  local pid=$!

  local deadline=$((SECONDS + startup_timeout))
  local ok=0
  while (( SECONDS < deadline )); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      echo "process exited early for ${broker}/${role}, log: $log_file" >&2
      tail -n 40 "$log_file" >&2 || true
      cleanup_pid "$pid"
      return 1
    fi

    if check_endpoint "$role" "$url_base" >/dev/null 2>&1; then
      ok=1
      break
    fi

    sleep "$poll_interval"
  done

  if [[ "$ok" -ne 1 ]]; then
    echo "startup timeout for ${broker}/${role}, log: $log_file" >&2
    tail -n 40 "$log_file" >&2 || true
    cleanup_pid "$pid"
    return 1
  fi

  echo "PASS ${broker}/${role} -> ${url_base}"
  cleanup_pid "$pid"
}

echo "Using OJ repository override: ${oj_repository}"
echo "Logs: ${log_dir}"

port="$base_port"
for broker in "${brokers[@]}"; do
  for role in "${roles[@]}"; do
    run_case "$broker" "$role" "$port"
    port=$((port + 1))
  done
done

echo "All runtime matrix cases passed."
