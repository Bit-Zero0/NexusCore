#[path = "support/broker_contract.rs"]
mod broker_contract;

use std::sync::Arc;

use broker_contract::{
    assert_dead_letter_persists_across_reconnect, assert_dead_letter_replay_is_consistent,
    assert_duplicate_delivery_is_eventually_singular, assert_publish_reserve_ack_roundtrip,
    assert_reclaims_unacked_delivery_after_reconnect, assert_reject_moves_to_dead_letter,
    assert_retry_dead_letter_and_replay_roundtrip, assert_retry_delay_boundary,
    assert_round_robin_across_routes, assert_route_fairness_jitter_stays_bounded,
    assert_stats_reflect_queue_states, BrokerContractHarness,
};
use nexus_runtime::{
    build_nats_runtime_queue, NatsQueueConfig, RuntimeRouteBinding, NATS_BROKER_CAPABILITIES,
};
use ulid::Ulid;

#[tokio::test]
async fn nats_publish_reserve_ack_roundtrip() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_publish_reserve_ack_roundtrip(harness).await;
}

#[tokio::test]
async fn nats_retry_dead_letter_and_replay_roundtrip() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_retry_dead_letter_and_replay_roundtrip(harness).await;
}

#[tokio::test]
async fn nats_dead_letter_persists_across_reconnect() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_dead_letter_persists_across_reconnect(harness).await;
}

#[tokio::test]
async fn nats_reject_moves_to_dead_letter() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_reject_moves_to_dead_letter(harness).await;
}

#[tokio::test]
async fn nats_stats_reflect_queue_states() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_stats_reflect_queue_states(harness).await;
}

#[tokio::test]
async fn nats_round_robin_across_routes() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_round_robin_across_routes(harness).await;
}

#[tokio::test]
async fn nats_reclaims_unacked_delivery_after_reconnect() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_reclaims_unacked_delivery_after_reconnect(harness).await;
}

#[tokio::test]
async fn nats_retry_delay_boundary() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_retry_delay_boundary(harness).await;
}

#[tokio::test]
async fn nats_route_fairness_jitter_stays_bounded() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_route_fairness_jitter_stays_bounded(harness).await;
}

#[tokio::test]
async fn nats_dead_letter_replay_is_consistent() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_dead_letter_replay_is_consistent(harness).await;
}

#[tokio::test]
async fn nats_duplicate_delivery_is_eventually_singular() {
    let Some(harness) = test_harness() else {
        return;
    };
    assert_duplicate_delivery_is_eventually_singular(harness).await;
}

fn test_harness() -> Option<BrokerContractHarness<NatsQueueConfig>> {
    let url = std::env::var("NEXUS_NATS_TEST_URL").ok()?;
    let suffix = Ulid::new().to_string().to_lowercase();
    let binding = RuntimeRouteBinding {
        queue: format!("it_queue_{suffix}"),
        lane: "fast".to_owned(),
    };
    let secondary_binding = RuntimeRouteBinding {
        queue: format!("it_queue_alt_{suffix}"),
        lane: "normal".to_owned(),
    };
    let config = NatsQueueConfig {
        url,
        stream_name: format!("NEXUS_RUNTIME_IT_{suffix}"),
        subject_prefix: format!("nexus.runtime.it.{suffix}"),
        consumer_prefix: format!("nexus-runtime-it-{suffix}"),
        ack_wait_ms: 200,
    };

    Some(BrokerContractHarness {
        config,
        binding,
        secondary_binding,
        retry_delay_ms: 100,
        capabilities: NATS_BROKER_CAPABILITIES,
        reclaim_wait_ms: 300,
        build_queue: Arc::new(|config| {
            Box::pin(async move { build_nats_runtime_queue(config).await.ok() })
        }),
    })
}
