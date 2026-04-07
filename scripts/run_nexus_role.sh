#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: $0 <broker:rabbitmq|nats|redis_streams> <role:embedded|gateway|runtime-worker> [mode:dev|monitoring]" >&2
  exit 1
fi

broker="$1"
role="$2"
mode="${3:-dev}"

case "$broker" in
  rabbitmq|nats|redis_streams) ;;
  *)
    echo "unsupported broker: $broker" >&2
    exit 1
    ;;
esac

case "$role" in
  embedded|gateway|runtime-worker) ;;
  *)
    echo "unsupported role: $role" >&2
    exit 1
    ;;
esac

case "$mode" in
  dev) env_file="env/dev.compose.${broker}.env" ;;
  monitoring) env_file="env/dev.compose.monitoring.${broker}.env" ;;
  *)
    echo "unsupported mode: $mode" >&2
    exit 1
    ;;
esac

if [[ ! -f "$env_file" ]]; then
  echo "missing env file: $env_file" >&2
  exit 1
fi

set -a
source "$env_file"
set +a

export NEXUS_PROCESS_ROLE="$role"

exec cargo run -p nexus-app
