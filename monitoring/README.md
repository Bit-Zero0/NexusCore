# Monitoring

Grafana dashboard:
- [monitoring/grafana/dashboards/nexus-runtime-broker-observability.json](/home/fmy/NexusCore/monitoring/grafana/dashboards/nexus-runtime-broker-observability.json)

Provisioning files:
- [monitoring/prometheus/prometheus.yml](/home/fmy/NexusCore/monitoring/prometheus/prometheus.yml)
- [monitoring/prometheus/rules/nexus-runtime-alerts.yml](/home/fmy/NexusCore/monitoring/prometheus/rules/nexus-runtime-alerts.yml)
- [monitoring/alertmanager/alertmanager.yml](/home/fmy/NexusCore/monitoring/alertmanager/alertmanager.yml)
- [main.rs](/home/fmy/NexusCore/crates/nexus-alert-relay/src/main.rs)
- [monitoring/grafana/provisioning/datasources/prometheus.yml](/home/fmy/NexusCore/monitoring/grafana/provisioning/datasources/prometheus.yml)
- [monitoring/grafana/provisioning/dashboards/dashboards.yml](/home/fmy/NexusCore/monitoring/grafana/provisioning/dashboards/dashboards.yml)

With `docker-compose.dev.yml`, Prometheus, Alertmanager, and Grafana can now start together with the local dependencies.

Broker native metrics:
- RabbitMQ is scraped from `http://127.0.0.1:15692/metrics`
- NATS is scraped through `prometheus-nats-exporter` on the compose network at `http://nats-exporter:7777/metrics`

Recommended scrape targets:
- `gateway` role: `/metrics`
- `runtime-worker` role: `/metrics`
- `embedded` role: `/metrics`

Runtime management API:
- `GET /api/v1/runtime/management/broker` exposes broker management state for control plane and ops UI
- `GET /api/v1/runtime/management/runbooks` exposes the stable runbook catalog for control plane jump links
- `summary.dead_letter_records_total` and `summary.replay_history_total` remain full filtered totals even when `dead_letters` and `replay_history` are paginated
- `health.status` exposes `healthy` or `degraded`
- `health.recovery_window_active` and `health.persistent_failures_detected` help distinguish brief recovery windows from sustained broker failures
- `health.alerts[].recommended_action.runbook` resolves `runbook_ref` into `title / doc_path / section_ref`

When Prometheus runs in Docker and your app runs on the host, bind the app to `0.0.0.0:8080` so `host.docker.internal:8080` is reachable from the container.

Monitoring-friendly env samples:
- [env/dev.compose.monitoring.rabbitmq.env](/home/fmy/NexusCore/env/dev.compose.monitoring.rabbitmq.env)
- [env/dev.compose.monitoring.nats.env](/home/fmy/NexusCore/env/dev.compose.monitoring.nats.env)
- [env/dev.compose.monitoring.redis_streams.env](/home/fmy/NexusCore/env/dev.compose.monitoring.redis_streams.env)

Dashboard focus:
- broker throughput and failure rate
- queue depth, leased tasks, dead letters
- retry / replay / reclaim activity
- broker capability and lease/reclaim settings
- 15 minute safety summary for retry / dead-letter / replay / reclaim

Prometheus alert rules:
- broker operation failures persisting for 10 minutes
- dead letters detected in the last 15 minutes
- reclaim activity spikes
- queued or delayed backlog staying high for 10 minutes

Local alert loop:
- Prometheus evaluates [nexus-runtime-alerts.yml](/home/fmy/NexusCore/monitoring/prometheus/rules/nexus-runtime-alerts.yml)
- Alertmanager receives alerts on `http://alertmanager:9093`
- Alertmanager forwards to the local relay on `http://alert-relay:18081/alertmanager`
- Grafana provisions both `Prometheus` and `Alertmanager` datasources
- alert-relay runtime status is available on `http://127.0.0.1:18081/status`

Notification channels:
- generic webhook receives the raw Alertmanager JSON payload
- Feishu bot webhook receives a formatted text message
- WeCom bot webhook receives a formatted markdown message
- local dry-run receiver logs formatted alert text and markdown for debugging

Severity routing:
- all alerts go to the generic webhook receiver
- `warning` alerts also go to Feishu
- `critical` alerts also go to WeCom

Dry-run debugging:
- Alertmanager also sends every alert to `http://alert-relay:18081/dry-run`
- `NEXUS_ALERT_DRY_RUN=true` lets the relay log alert content even when no real webhook URL is configured
- `NEXUS_ALERT_AUDIT_LOG_PATH` writes JSONL audit records for alert history
- `NEXUS_ALERT_AUDIT_LOG_MAX_BYTES` rolls over the active audit file after it grows past the configured size
- `NEXUS_ALERT_AUDIT_LOG_MAX_FILES` keeps only the most recent rotated audit files
- `NEXUS_ALERT_AUDIT_LOG_RETENTION_DAYS` prunes rotated audit files older than the configured number of days
- [query_alert_audit.py](/home/fmy/NexusCore/scripts/query_alert_audit.py) filters and exports audit JSONL records without manual file parsing

Audit query examples:
```bash
python3 scripts/query_alert_audit.py --path /tmp/nexus-alert-relay/audit.jsonl --severity warning
python3 scripts/query_alert_audit.py --path /tmp/nexus-alert-relay --channel dry-run --format json
python3 scripts/query_alert_audit.py --alertname NexusBrokerDeadLettersDetected --export /tmp/dead-letters.jsonl
python3 scripts/query_alert_audit.py --since 15m --contains reclaim
python3 scripts/query_alert_audit.py --since 2026-04-07T07:00:00+08:00 --contains rabbitmq
python3 scripts/query_alert_audit.py --since 1d --summary alertname
python3 scripts/query_alert_audit.py --since 1d --summary severity --format json
```

Alerting env sample:
- [env/alerting.local.env](/home/fmy/NexusCore/env/alerting.local.env)
- [prod.alerting.env.example](/home/fmy/NexusCore/env/prod.alerting.env.example)

P0 runbook:
- [P0_生产稳定性与运维闭环_Runbook.md](/home/fmy/NexusCore/P0_生产稳定性与运维闭环_Runbook.md)
- [生产部署与配置收口手册.md](/home/fmy/NexusCore/生产部署与配置收口手册.md)

P0 scripts:
- [verify_alert_pipeline.sh](/home/fmy/NexusCore/scripts/verify_alert_pipeline.sh)
- [run_broker_failure_drill.sh](/home/fmy/NexusCore/scripts/run_broker_failure_drill.sh)
- [query_alert_audit.py](/home/fmy/NexusCore/scripts/query_alert_audit.py)
- [check_deploy_config.sh](/home/fmy/NexusCore/scripts/check_deploy_config.sh)
