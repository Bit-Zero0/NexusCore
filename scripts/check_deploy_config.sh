#!/usr/bin/env bash
set -euo pipefail

role="${NEXUS_PROCESS_ROLE:-embedded}"
broker="${NEXUS_RUNTIME_BROKER_BACKEND:-memory}"
repository="${NEXUS_OJ_REPOSITORY:-memory}"

errors=()
warnings=()

require_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    errors+=("missing required env: ${name}")
  fi
}

warn_if_equals() {
  local name="$1"
  local value="$2"
  local message="$3"
  if [[ "${!name:-}" == "$value" ]]; then
    warnings+=("${message}")
  fi
}

require_var NEXUS_ENV
require_var NEXUS_PROCESS_ROLE
require_var NEXUS_BIND_ADDR
require_var NEXUS_RUNTIME_BROKER_BACKEND
require_var NEXUS_RUNTIME_NODE_ID
require_var NEXUS_RUNTIME_WORK_ROOT
require_var NEXUS_RUNTIME_NSJAIL_PATH
require_var NEXUS_RUNTIME_SECCOMP_MODE

case "$role" in
  embedded|gateway|runtime-worker) ;;
  *) errors+=("unsupported NEXUS_PROCESS_ROLE: ${role}") ;;
esac

case "$broker" in
  memory)
    warnings+=("broker backend is memory; this is only suitable for local development")
    ;;
  rabbitmq)
    require_var NEXUS_RUNTIME_RABBITMQ_URL
    require_var NEXUS_RUNTIME_RABBITMQ_EXCHANGE
    require_var NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX
    ;;
  nats)
    require_var NEXUS_RUNTIME_NATS_URL
    require_var NEXUS_RUNTIME_NATS_STREAM
    require_var NEXUS_RUNTIME_NATS_SUBJECT_PREFIX
    require_var NEXUS_RUNTIME_NATS_CONSUMER_PREFIX
    require_var NEXUS_RUNTIME_NATS_ACK_WAIT_MS
    ;;
  redis_streams)
    require_var NEXUS_RUNTIME_REDIS_STREAMS_URL
    require_var NEXUS_RUNTIME_REDIS_STREAMS_PREFIX
    require_var NEXUS_RUNTIME_REDIS_STREAMS_GROUP_PREFIX
    require_var NEXUS_RUNTIME_REDIS_STREAMS_CONSUMER_PREFIX
    require_var NEXUS_RUNTIME_REDIS_STREAMS_PENDING_RECLAIM_IDLE_MS
    ;;
  *)
    errors+=("unsupported NEXUS_RUNTIME_BROKER_BACKEND: ${broker}")
    ;;
esac

case "$repository" in
  postgres)
    require_var NEXUS_PG_HOST
    require_var NEXUS_PG_PORT
    require_var NEXUS_PG_DATABASE
    require_var NEXUS_PG_USERNAME
    require_var NEXUS_PG_PASSWORD
    require_var NEXUS_PG_MAX_CONNECTIONS
    ;;
  memory)
    warnings+=("OJ repository is memory; persistent submission/problem data will not survive restart")
    ;;
  *)
    errors+=("unsupported NEXUS_OJ_REPOSITORY: ${repository}")
    ;;
esac

require_var NEXUS_REDIS_URL
require_var NEXUS_LOG_FORMAT

warn_if_equals NEXUS_ENV dev "NEXUS_ENV is dev; production deployment should use NEXUS_ENV=prod"
warn_if_equals NEXUS_RUNTIME_SECCOMP_MODE log "NEXUS_RUNTIME_SECCOMP_MODE is log; production should prefer kill"

if [[ "${NEXUS_LOG_FORMAT:-}" != "json" ]]; then
  warnings+=("NEXUS_LOG_FORMAT is not json; structured production logs are recommended")
fi

if [[ ${#warnings[@]} -gt 0 ]]; then
  echo "Warnings:"
  for warning in "${warnings[@]}"; do
    echo "  - ${warning}"
  done
fi

if [[ ${#errors[@]} -gt 0 ]]; then
  echo "Errors:"
  for error in "${errors[@]}"; do
    echo "  - ${error}"
  done
  exit 1
fi

echo "Deployment configuration check passed."
echo "Role: ${role}"
echo "Broker: ${broker}"
echo "Repository: ${repository}"
