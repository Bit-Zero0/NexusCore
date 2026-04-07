#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

mode="${VERIFY_ALERT_MODE:-local}"
capture_dir="${CAPTURE_DIR:-/tmp/nexus-alert-capture}"
capture_port="${CAPTURE_PORT:-18082}"
relay_port="${RELAY_PORT:-18081}"
alertmanager_url="${ALERTMANAGER_URL:-http://127.0.0.1:9093}"
alertmanager_port="${ALERTMANAGER_PORT:-19093}"
wait_seconds="${WAIT_SECONDS:-50}"
audit_dir="${AUDIT_DIR:-/tmp/nexus-alert-relay}"
audit_path="${NEXUS_ALERT_AUDIT_LOG_PATH:-${audit_dir}/audit.jsonl}"

mkdir -p "$capture_dir" "$audit_dir"
rm -f "$capture_dir"/*.json "$capture_dir"/server.log "$audit_dir"/*.jsonl "$audit_dir"/audit*.jsonl

cleanup() {
  if [[ -n "${alertmanager_container:-}" ]]; then
    docker rm -f "$alertmanager_container" >/dev/null 2>&1 || true
  fi
  if [[ -n "${relay_pid:-}" ]] && kill -0 "$relay_pid" >/dev/null 2>&1; then
    kill "$relay_pid" >/dev/null 2>&1 || true
    wait "$relay_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "${capture_pid:-}" ]] && kill -0 "$capture_pid" >/dev/null 2>&1; then
    kill "$capture_pid" >/dev/null 2>&1 || true
    wait "$capture_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

python3 - "$capture_dir" "$capture_port" >"$capture_dir/server.log" 2>&1 <<'PY' &
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

capture_dir = Path(sys.argv[1])
port = int(sys.argv[2])

class Handler(BaseHTTPRequestHandler):
    counter = 0

    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8")
        Handler.counter += 1
        name = self.path.strip("/").replace("/", "_") or "root"
        target = capture_dir / f"{Handler.counter:02d}-{name}.json"
        target.write_text(body, encoding="utf-8")
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, fmt, *args):
        return

HTTPServer(("127.0.0.1", port), Handler).serve_forever()
PY
capture_pid=$!

timeout 30 bash -c "until curl -fsS http://127.0.0.1:${capture_port}/ >/dev/null; do sleep 1; done"

export NEXUS_ALERT_WEBHOOK_URL="http://127.0.0.1:${capture_port}/generic"
export NEXUS_ALERT_FEISHU_WEBHOOK_URL="http://127.0.0.1:${capture_port}/feishu"
export NEXUS_ALERT_WECOM_WEBHOOK_URL="http://127.0.0.1:${capture_port}/wecom"
export NEXUS_ALERT_DRY_RUN=true
export NEXUS_ALERT_AUDIT_LOG_PATH="$audit_path"
export NEXUS_ALERT_RELAY_BIND_ADDR="127.0.0.1:${relay_port}"

cargo run -p nexus-alert-relay >"${capture_dir}/relay.log" 2>&1 &
relay_pid=$!

timeout 60 bash -c "until curl -fsS http://127.0.0.1:${relay_port}/healthz >/dev/null; do sleep 2; done"

warning_payload='{"status":"firing","alerts":[{"labels":{"alertname":"P0WarningProbe","severity":"warning","broker":"rabbitmq","queue":"probe","lane":"default"},"annotations":{"summary":"warning delivery smoke"}}]}'
critical_payload='{"status":"firing","alerts":[{"labels":{"alertname":"P0CriticalProbe","severity":"critical","broker":"nats","queue":"probe","lane":"default"},"annotations":{"summary":"critical delivery smoke"}}]}'
warning_alert='[{"labels":{"alertname":"P0WarningProbe","severity":"warning","broker":"rabbitmq","queue":"probe","lane":"default"},"annotations":{"summary":"warning delivery smoke"}}]'
critical_alert='[{"labels":{"alertname":"P0CriticalProbe","severity":"critical","broker":"nats","queue":"probe","lane":"default"},"annotations":{"summary":"critical delivery smoke"}}]'

if [[ "$mode" == "alertmanager" ]]; then
  alertmanager_container="nexus-alertmanager-p0"
  alertmanager_url="${ALERTMANAGER_URL:-http://127.0.0.1:${alertmanager_port}}"
  docker rm -f "$alertmanager_container" >/dev/null 2>&1 || true
  docker run -d \
    --name "$alertmanager_container" \
    --add-host host.docker.internal:host-gateway \
    -p "${alertmanager_port}:9093" \
    -v "$ROOT_DIR/monitoring/alertmanager/alertmanager.host.yml:/etc/alertmanager/alertmanager.yml:ro" \
    prom/alertmanager:v0.28.1 \
    --config.file=/etc/alertmanager/alertmanager.yml \
    --storage.path=/alertmanager >/dev/null
  timeout 120 bash -c "until curl -fsS ${alertmanager_url}/-/healthy >/dev/null; do sleep 2; done"
  curl -fsS -X POST "${alertmanager_url}/api/v2/alerts" \
    -H 'Content-Type: application/json' \
    --data "$warning_alert" >/dev/null
  curl -fsS -X POST "${alertmanager_url}/api/v2/alerts" \
    -H 'Content-Type: application/json' \
    --data "$critical_alert" >/dev/null
else
  curl -fsS -X POST "http://127.0.0.1:${relay_port}/generic" \
    -H 'Content-Type: application/json' \
    --data "$warning_payload" >/dev/null
  curl -fsS -X POST "http://127.0.0.1:${relay_port}/feishu" \
    -H 'Content-Type: application/json' \
    --data "$warning_payload" >/dev/null
  curl -fsS -X POST "http://127.0.0.1:${relay_port}/wecom" \
    -H 'Content-Type: application/json' \
    --data "$critical_payload" >/dev/null
  curl -fsS -X POST "http://127.0.0.1:${relay_port}/dry-run" \
    -H 'Content-Type: application/json' \
    --data "$critical_payload" >/dev/null
fi

deadline=$((SECONDS + wait_seconds))
while (( SECONDS < deadline )); do
  generic_count=$(find "$capture_dir" -maxdepth 1 -name '*-generic.json' | wc -l | tr -d ' ')
  feishu_count=$(find "$capture_dir" -maxdepth 1 -name '*-feishu.json' | wc -l | tr -d ' ')
  wecom_count=$(find "$capture_dir" -maxdepth 1 -name '*-wecom.json' | wc -l | tr -d ' ')
  if [[ -f "$audit_path" && "$generic_count" -ge 1 && "$feishu_count" -ge 1 && "$wecom_count" -ge 1 ]]; then
    break
  fi
  sleep 2
done

generic_count=$(find "$capture_dir" -maxdepth 1 -name '*-generic.json' | wc -l | tr -d ' ')
feishu_count=$(find "$capture_dir" -maxdepth 1 -name '*-feishu.json' | wc -l | tr -d ' ')
wecom_count=$(find "$capture_dir" -maxdepth 1 -name '*-wecom.json' | wc -l | tr -d ' ')

if [[ "$generic_count" -lt 1 ]]; then
  echo "expected generic receiver to capture at least one alert" >&2
  exit 1
fi
if [[ "$feishu_count" -lt 1 ]]; then
  echo "expected warning alert to reach feishu receiver" >&2
  exit 1
fi
if [[ "$wecom_count" -lt 1 ]]; then
  echo "expected critical alert to reach wecom receiver" >&2
  exit 1
fi
if [[ ! -f "$audit_path" ]]; then
  echo "expected audit log to be written to ${audit_path}" >&2
  exit 1
fi

echo "Alert delivery verified in ${mode} mode."
echo "Captured payloads:"
find "$capture_dir" -maxdepth 1 -type f -name '*.json' | sort
echo "Audit log:"
echo "$audit_path"
