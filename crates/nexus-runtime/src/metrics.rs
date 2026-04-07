use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use once_cell::sync::Lazy;
use prometheus::{
    register_int_counter_vec_with_registry, register_int_gauge_vec_with_registry, Encoder,
    IntCounterVec, IntGaugeVec, Registry, TextEncoder,
};

use nexus_shared::{AppError, AppResult};

use crate::{RuntimeBrokerObservabilityStatus, RuntimeTaskService};

static REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

static BROKER_INFO: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec_with_registry!(
        "nexus_runtime_broker_info",
        "Runtime broker identity for the current node",
        &["node_id", "broker"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_info")
});

static WORKER_GROUP_COUNT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec_with_registry!(
        "nexus_runtime_worker_group_count",
        "Configured runtime worker group count for the current node",
        &["node_id"],
        &REGISTRY
    )
    .expect("register nexus_runtime_worker_group_count")
});

static QUEUE_DEPTH: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec_with_registry!(
        "nexus_runtime_queue_depth",
        "Runtime broker queue depth by state",
        &["node_id", "broker", "queue", "lane", "state"],
        &REGISTRY
    )
    .expect("register nexus_runtime_queue_depth")
});

static BROKER_REQUIRED_CAPABILITY: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec_with_registry!(
        "nexus_runtime_broker_required_capability",
        "Required broker capability availability",
        &["node_id", "broker", "capability"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_required_capability")
});

static BROKER_ENHANCED_CAPABILITY: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec_with_registry!(
        "nexus_runtime_broker_enhanced_capability",
        "Enhanced broker capability availability",
        &["node_id", "broker", "capability"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_enhanced_capability")
});

static BROKER_SETTING_MS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec_with_registry!(
        "nexus_runtime_broker_setting_milliseconds",
        "Broker lease or reclaim timing settings in milliseconds",
        &["node_id", "broker", "setting"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_setting_milliseconds")
});

static BROKER_RECLAIM_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_reclaim_total",
        "Number of task leases reclaimed by the broker",
        &["broker", "queue", "lane"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_reclaim_total")
});

static BROKER_RECLAIM_ORPHAN_CLEANUP_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_reclaim_orphan_cleanup_total",
        "Number of orphaned pending entries cleaned during broker reclaim",
        &["broker", "queue", "lane"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_reclaim_orphan_cleanup_total")
});

static BROKER_RETRY_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_retry_total",
        "Number of broker retry operations by disposition",
        &["broker", "queue", "lane", "disposition"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_retry_total")
});

static BROKER_DEAD_LETTER_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_dead_letter_total",
        "Number of broker dead-letter operations by reason",
        &["broker", "queue", "lane", "reason"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_dead_letter_total")
});

static BROKER_REPLAY_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_replay_total",
        "Number of broker dead-letter replay operations",
        &["broker", "queue", "lane"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_replay_total")
});

static BROKER_OPERATION_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_operation_total",
        "Number of successful broker operations",
        &["broker", "queue", "lane", "operation"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_operation_total")
});

static BROKER_OPERATION_FAILURE_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "nexus_runtime_broker_operation_failure_total",
        "Number of failed broker operations",
        &["broker", "queue", "lane", "operation"],
        &REGISTRY
    )
    .expect("register nexus_runtime_broker_operation_failure_total")
});

static BROKER_FAILURE_WINDOWS: Lazy<Mutex<HashMap<String, VecDeque<u64>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

const BROKER_FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);
const BROKER_RECOVERY_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default)]
pub struct BrokerFailureHealthSnapshot {
    pub last_failure_at_ms: Option<u64>,
    pub recent_failure_count: u64,
    pub recovery_window_active: bool,
    pub persistent_failures_detected: bool,
}

pub fn observe_broker_reclaim(broker: &str, queue: &str, lane: &str) {
    BROKER_RECLAIM_TOTAL
        .with_label_values(&[broker, queue, lane])
        .inc();
}

pub fn observe_broker_operation(broker: &str, queue: &str, lane: &str, operation: &str) {
    BROKER_OPERATION_TOTAL
        .with_label_values(&[broker, queue, lane, operation])
        .inc();
}

pub fn observe_broker_operation_failure(broker: &str, queue: &str, lane: &str, operation: &str) {
    BROKER_OPERATION_FAILURE_TOTAL
        .with_label_values(&[broker, queue, lane, operation])
        .inc();
    record_broker_failure(broker);
}

pub fn observe_broker_retry(broker: &str, queue: &str, lane: &str, disposition: &str) {
    BROKER_RETRY_TOTAL
        .with_label_values(&[broker, queue, lane, disposition])
        .inc();
}

pub fn observe_broker_dead_letter(broker: &str, queue: &str, lane: &str, reason: &str) {
    BROKER_DEAD_LETTER_TOTAL
        .with_label_values(&[broker, queue, lane, reason])
        .inc();
}

pub fn observe_broker_replay(broker: &str, queue: &str, lane: &str) {
    BROKER_REPLAY_TOTAL
        .with_label_values(&[broker, queue, lane])
        .inc();
}

pub fn observe_broker_reclaim_orphan_cleanup(
    broker: &str,
    queue: &str,
    lane: &str,
    orphan_count: u64,
) {
    if orphan_count == 0 {
        return;
    }
    BROKER_RECLAIM_ORPHAN_CLEANUP_TOTAL
        .with_label_values(&[broker, queue, lane])
        .inc_by(orphan_count);
}

pub fn broker_failure_health_snapshot(broker: &str) -> BrokerFailureHealthSnapshot {
    let now_ms = now_ms();
    let cutoff = now_ms.saturating_sub(BROKER_FAILURE_WINDOW.as_millis() as u64);
    let mut windows = BROKER_FAILURE_WINDOWS
        .lock()
        .expect("broker failure health mutex poisoned");
    let Some(entries) = windows.get_mut(broker) else {
        return BrokerFailureHealthSnapshot::default();
    };
    prune_failure_window(entries, cutoff);
    let last_failure_at_ms = entries.back().copied();
    let recent_failure_count = entries.len() as u64;
    let recovery_window_active = last_failure_at_ms
        .map(|ts| now_ms.saturating_sub(ts) <= BROKER_RECOVERY_WINDOW.as_millis() as u64)
        .unwrap_or(false);
    let persistent_failures_detected = recent_failure_count >= 3;
    BrokerFailureHealthSnapshot {
        last_failure_at_ms,
        recent_failure_count,
        recovery_window_active,
        persistent_failures_detected,
    }
}

fn record_broker_failure(broker: &str) {
    let now_ms = now_ms();
    let cutoff = now_ms.saturating_sub(BROKER_FAILURE_WINDOW.as_millis() as u64);
    let mut windows = BROKER_FAILURE_WINDOWS
        .lock()
        .expect("broker failure health mutex poisoned");
    let entries = windows.entry(broker.to_owned()).or_default();
    entries.push_back(now_ms);
    prune_failure_window(entries, cutoff);
}

fn prune_failure_window(entries: &mut VecDeque<u64>, cutoff_ms: u64) {
    while entries.front().is_some_and(|value| *value < cutoff_ms) {
        entries.pop_front();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub async fn render_prometheus_metrics(service: &RuntimeTaskService) -> AppResult<String> {
    Lazy::force(&BROKER_INFO);
    Lazy::force(&WORKER_GROUP_COUNT);
    Lazy::force(&QUEUE_DEPTH);
    Lazy::force(&BROKER_REQUIRED_CAPABILITY);
    Lazy::force(&BROKER_ENHANCED_CAPABILITY);
    Lazy::force(&BROKER_SETTING_MS);
    Lazy::force(&BROKER_RECLAIM_TOTAL);
    Lazy::force(&BROKER_RECLAIM_ORPHAN_CLEANUP_TOTAL);
    Lazy::force(&BROKER_RETRY_TOTAL);
    Lazy::force(&BROKER_DEAD_LETTER_TOTAL);
    Lazy::force(&BROKER_REPLAY_TOTAL);
    Lazy::force(&BROKER_OPERATION_TOTAL);
    Lazy::force(&BROKER_OPERATION_FAILURE_TOTAL);

    let node_status = service.node_status();
    let broker_status = node_status.broker.clone();
    let queue_stats = service.queue_stats().await?;
    let node_id = node_status.node_id.as_str();
    let broker = broker_status.broker.as_str();

    BROKER_INFO.reset();
    BROKER_INFO.with_label_values(&[node_id, broker]).set(1);

    WORKER_GROUP_COUNT.reset();
    WORKER_GROUP_COUNT
        .with_label_values(&[node_id])
        .set(node_status.worker_groups.len() as i64);

    QUEUE_DEPTH.reset();
    for stat in queue_stats {
        set_queue_depth_metric(
            node_id,
            broker,
            &stat.queue,
            &stat.lane,
            "queued",
            stat.queued,
        );
        set_queue_depth_metric(
            node_id,
            broker,
            &stat.queue,
            &stat.lane,
            "leased",
            stat.leased,
        );
        set_queue_depth_metric(
            node_id,
            broker,
            &stat.queue,
            &stat.lane,
            "dead_lettered",
            stat.dead_lettered,
        );
    }

    BROKER_REQUIRED_CAPABILITY.reset();
    set_required_capability_metrics(node_id, &broker_status);

    BROKER_ENHANCED_CAPABILITY.reset();
    set_enhanced_capability_metrics(node_id, &broker_status);

    BROKER_SETTING_MS.reset();
    if let Some(ack_wait_ms) = broker_status.ack_wait_ms {
        BROKER_SETTING_MS
            .with_label_values(&[node_id, broker, "ack_wait_ms"])
            .set(ack_wait_ms as i64);
    }
    if let Some(pending_reclaim_idle_ms) = broker_status.pending_reclaim_idle_ms {
        BROKER_SETTING_MS
            .with_label_values(&[node_id, broker, "pending_reclaim_idle_ms"])
            .set(pending_reclaim_idle_ms as i64);
    }

    let metric_families = REGISTRY.gather();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|_| AppError::Internal)?;
    String::from_utf8(buffer).map_err(|_| AppError::Internal)
}

fn set_queue_depth_metric(
    node_id: &str,
    broker: &str,
    queue: &str,
    lane: &str,
    state: &str,
    value: usize,
) {
    QUEUE_DEPTH
        .with_label_values(&[node_id, broker, queue, lane, state])
        .set(value as i64);
}

fn set_required_capability_metrics(
    node_id: &str,
    broker_status: &RuntimeBrokerObservabilityStatus,
) {
    let broker = broker_status.broker.as_str();
    BROKER_REQUIRED_CAPABILITY
        .with_label_values(&[node_id, broker, "enqueue"])
        .set(bool_to_gauge(broker_status.required_capabilities.enqueue));
    BROKER_REQUIRED_CAPABILITY
        .with_label_values(&[node_id, broker, "reserve"])
        .set(bool_to_gauge(broker_status.required_capabilities.reserve));
    BROKER_REQUIRED_CAPABILITY
        .with_label_values(&[node_id, broker, "ack"])
        .set(bool_to_gauge(broker_status.required_capabilities.ack));
    BROKER_REQUIRED_CAPABILITY
        .with_label_values(&[node_id, broker, "retry"])
        .set(bool_to_gauge(broker_status.required_capabilities.retry));
    BROKER_REQUIRED_CAPABILITY
        .with_label_values(&[node_id, broker, "reject"])
        .set(bool_to_gauge(broker_status.required_capabilities.reject));
}

fn set_enhanced_capability_metrics(
    node_id: &str,
    broker_status: &RuntimeBrokerObservabilityStatus,
) {
    let broker = broker_status.broker.as_str();
    BROKER_ENHANCED_CAPABILITY
        .with_label_values(&[node_id, broker, "stats"])
        .set(bool_to_gauge(broker_status.enhanced_capabilities.stats));
    BROKER_ENHANCED_CAPABILITY
        .with_label_values(&[node_id, broker, "dead_letter_store"])
        .set(bool_to_gauge(
            broker_status.enhanced_capabilities.dead_letter_store,
        ));
    BROKER_ENHANCED_CAPABILITY
        .with_label_values(&[node_id, broker, "dead_letter_replay"])
        .set(bool_to_gauge(
            broker_status.enhanced_capabilities.dead_letter_replay,
        ));
    BROKER_ENHANCED_CAPABILITY
        .with_label_values(&[node_id, broker, "route_fairness"])
        .set(bool_to_gauge(
            broker_status.enhanced_capabilities.route_fairness,
        ));
    BROKER_ENHANCED_CAPABILITY
        .with_label_values(&[node_id, broker, "crash_reclaim"])
        .set(bool_to_gauge(
            broker_status.enhanced_capabilities.crash_reclaim,
        ));
}

fn bool_to_gauge(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        build_default_runtime_catalog, NoopRuntimeEventObserver, RuntimeSeccompMode,
        RuntimeSyscallFlavor, RuntimeTaskService, RuntimeWorker,
    };

    use super::{
        observe_broker_dead_letter, observe_broker_operation, observe_broker_operation_failure,
        observe_broker_replay, observe_broker_retry, render_prometheus_metrics,
    };

    #[tokio::test]
    async fn renders_prometheus_metrics_for_runtime_service() {
        let service = RuntimeTaskService::new(
            Arc::new(RuntimeWorker::new(
                build_default_runtime_catalog(),
                "/tmp/nexus-runtime-metrics-test",
                "/usr/bin/nsjail",
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                crate::debug_detected_runtime_syscall_arch(),
            )),
            Arc::new(NoopRuntimeEventObserver),
        );
        service.register_node("runtime-node-metrics");
        observe_broker_operation("memory", "oj_judge", "fast", "enqueue");
        observe_broker_operation("memory", "oj_judge", "fast", "reserve");
        observe_broker_operation("memory", "oj_judge", "fast", "ack");
        observe_broker_operation_failure("memory", "oj_judge", "fast", "ack");
        observe_broker_retry("memory", "oj_judge", "fast", "requeued");
        observe_broker_dead_letter("memory", "oj_judge", "fast", "rejected");
        observe_broker_replay("memory", "oj_judge", "fast");

        let rendered = render_prometheus_metrics(&service)
            .await
            .expect("metrics should render");

        assert!(rendered.contains("nexus_runtime_broker_info"));
        assert!(rendered.contains("nexus_runtime_worker_group_count"));
        assert!(rendered.contains("nexus_runtime_broker_retry_total"));
        assert!(rendered.contains("nexus_runtime_broker_dead_letter_total"));
        assert!(rendered.contains("nexus_runtime_broker_replay_total"));
        assert!(rendered.contains("nexus_runtime_broker_operation_total"));
        assert!(rendered.contains("nexus_runtime_broker_operation_failure_total"));
        assert!(rendered.contains("runtime-node-metrics"));
    }
}
