use std::{future::Future, pin::Pin, sync::Arc};

use nexus_runtime::{
    BrokerCapabilityProfile, OjJudgeTask, RuntimeJudgeMode, RuntimeLimits, RuntimeRetryPolicy,
    RuntimeRouteBinding, RuntimeSandboxKind, RuntimeTask, RuntimeTaskDelivery, RuntimeTaskPayload,
    RuntimeTaskQueue, RuntimeTaskType, RuntimeTestcase,
};
use nexus_shared::{ProblemId, SubmissionId, UserId};
use tokio::time::{sleep, Duration, Instant};
use ulid::Ulid;

type BuildQueueFuture =
    Pin<Box<dyn Future<Output = Option<Arc<dyn RuntimeTaskQueue>>> + Send + 'static>>;

pub struct BrokerContractHarness<C> {
    pub config: C,
    pub binding: RuntimeRouteBinding,
    pub secondary_binding: RuntimeRouteBinding,
    pub build_queue: Arc<dyn Fn(C) -> BuildQueueFuture + Send + Sync>,
    pub retry_delay_ms: u64,
    pub capabilities: BrokerCapabilityProfile,
    pub reclaim_wait_ms: u64,
}

impl<C> BrokerContractHarness<C>
where
    C: Clone + Send + Sync + 'static,
{
    pub async fn build_queue(&self) -> Option<Arc<dyn RuntimeTaskQueue>> {
        (self.build_queue)(self.config.clone()).await
    }
}

pub async fn assert_publish_reserve_ack_roundtrip<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        3,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");

    assert_eq!(delivery.task.task_id, task.task_id);
    assert_eq!(delivery.attempt, 1);
    queue
        .ack(&delivery.delivery_id)
        .await
        .expect("ack should succeed");
}

pub async fn assert_retry_dead_letter_and_replay_roundtrip<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        2,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");

    let first = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("first delivery should exist");
    let disposition = queue
        .retry(
            &first.delivery_id,
            "transient failure",
            harness.retry_delay_ms,
        )
        .await
        .expect("retry should succeed");
    assert_eq!(format!("{disposition:?}"), "Requeued");

    let second = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(5),
    )
    .await
    .expect("second delivery should exist");
    assert_eq!(second.attempt, 2);
    assert_eq!(second.last_error.as_deref(), Some("transient failure"));

    let disposition = queue
        .retry(
            &second.delivery_id,
            "persistent failure",
            harness.retry_delay_ms,
        )
        .await
        .expect("retry should move to dead letter");
    assert_eq!(format!("{disposition:?}"), "DeadLettered");

    let dead_letters = queue
        .dead_letters()
        .await
        .expect("dead letters should be readable");
    assert_eq!(dead_letters.len(), 1);
    assert_eq!(dead_letters[0].task_id, task.task_id);

    queue
        .replay_dead_letter(&dead_letters[0].delivery_id)
        .await
        .expect("replay should succeed");

    let replayed = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("replayed delivery should exist");
    assert_eq!(replayed.attempt, 1);
    assert!(replayed
        .last_error
        .as_deref()
        .is_some_and(|value| value.contains("replayed from dead letter")));
}

pub async fn assert_retry_delay_boundary<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let delay_ms = harness.retry_delay_ms.max(200);
    let task = runtime_task(&harness.binding.queue, &harness.binding.lane, 3, delay_ms);

    queue.enqueue(task).await.expect("enqueue should succeed");
    let first = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");
    queue
        .retry(&first.delivery_id, "boundary retry", delay_ms)
        .await
        .expect("retry should succeed");

    let early = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_millis(delay_ms / 3),
    )
    .await;
    assert!(
        early.is_none(),
        "delivery should not be visible before retry delay elapses"
    );

    let retried = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_millis(delay_ms * 2),
    )
    .await
    .expect("delivery should become visible after retry delay");
    assert_eq!(retried.attempt, 2);
    queue
        .ack(&retried.delivery_id)
        .await
        .expect("ack should succeed");
}

pub async fn assert_dead_letter_persists_across_reconnect<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        1,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");
    let disposition = queue
        .retry(
            &delivery.delivery_id,
            "persistent failure",
            harness.retry_delay_ms,
        )
        .await
        .expect("retry should move to dead letter");
    assert_eq!(format!("{disposition:?}"), "DeadLettered");

    drop(queue);

    let reopened = harness.build_queue().await.expect("queue should rebuild");
    let dead_letters = reopened
        .dead_letters()
        .await
        .expect("dead letters should be readable");
    assert_eq!(dead_letters.len(), 1);
    assert_eq!(dead_letters[0].task_id, task.task_id);
}

pub async fn assert_reject_moves_to_dead_letter<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        3,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");

    queue
        .reject(&delivery.delivery_id, "manual rejection")
        .await
        .expect("reject should succeed");

    let dead_letters = queue
        .dead_letters()
        .await
        .expect("dead letters should be readable");
    assert_eq!(dead_letters.len(), 1);
    assert_eq!(dead_letters[0].task_id, task.task_id);
    assert_eq!(dead_letters[0].error, "manual rejection");

    let redelivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_millis(300),
    )
    .await;
    assert!(
        redelivery.is_none(),
        "rejected task should not be re-delivered"
    );
}

pub async fn assert_stats_reflect_queue_states<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };

    let leased_task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        3,
        harness.retry_delay_ms,
    );
    let queued_task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        3,
        harness.retry_delay_ms,
    );
    let dead_letter_task = runtime_task(
        &harness.secondary_binding.queue,
        &harness.secondary_binding.lane,
        3,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(leased_task.clone())
        .await
        .expect("enqueue should succeed");
    queue
        .enqueue(queued_task.clone())
        .await
        .expect("enqueue should succeed");
    queue
        .enqueue(dead_letter_task.clone())
        .await
        .expect("enqueue should succeed");

    let leased = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("leased delivery should exist");

    let to_reject = reserve_until(
        &queue,
        std::slice::from_ref(&harness.secondary_binding),
        Duration::from_secs(3),
    )
    .await
    .expect("secondary delivery should exist");
    queue
        .reject(&to_reject.delivery_id, "stats dead letter")
        .await
        .expect("reject should succeed");

    let stats = queue_stats_until(
        &queue,
        |stats| {
            stats_for(stats, &harness.binding)
                .is_some_and(|stat| stat.leased == 1 && stat.queued == 1)
                && stats_for(stats, &harness.secondary_binding)
                    .is_some_and(|stat| stat.dead_lettered == 1)
        },
        Duration::from_secs(3),
    )
    .await
    .expect("stats should reflect leased and dead-lettered counts");

    let primary = stats_for(&stats, &harness.binding).expect("primary stats should exist");
    assert_eq!(primary.queued, 1);
    assert_eq!(primary.leased, 1);
    assert_eq!(primary.dead_lettered, 0);

    let secondary =
        stats_for(&stats, &harness.secondary_binding).expect("secondary stats should exist");
    assert_eq!(secondary.queued, 0);
    assert_eq!(secondary.leased, 0);
    assert_eq!(secondary.dead_lettered, 1);

    queue
        .ack(&leased.delivery_id)
        .await
        .expect("ack should succeed");
}

pub async fn assert_round_robin_across_routes<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };

    let tasks = vec![
        runtime_task(
            &harness.binding.queue,
            &harness.binding.lane,
            3,
            harness.retry_delay_ms,
        ),
        runtime_task(
            &harness.binding.queue,
            &harness.binding.lane,
            3,
            harness.retry_delay_ms,
        ),
        runtime_task(
            &harness.secondary_binding.queue,
            &harness.secondary_binding.lane,
            3,
            harness.retry_delay_ms,
        ),
        runtime_task(
            &harness.secondary_binding.queue,
            &harness.secondary_binding.lane,
            3,
            harness.retry_delay_ms,
        ),
    ];

    queue
        .enqueue(tasks[0].clone())
        .await
        .expect("enqueue should succeed");
    queue
        .enqueue(tasks[1].clone())
        .await
        .expect("enqueue should succeed");
    queue
        .enqueue(tasks[2].clone())
        .await
        .expect("enqueue should succeed");
    queue
        .enqueue(tasks[3].clone())
        .await
        .expect("enqueue should succeed");

    let bindings = vec![harness.binding.clone(), harness.secondary_binding.clone()];
    let mut seen = Vec::new();
    for _ in 0..4 {
        let delivery = reserve_until(&queue, &bindings, Duration::from_secs(3))
            .await
            .expect("delivery should exist");
        seen.push((delivery.task.queue.clone(), delivery.task.lane.clone()));
        queue
            .ack(&delivery.delivery_id)
            .await
            .expect("ack should succeed");
    }

    let first = (harness.binding.queue.clone(), harness.binding.lane.clone());
    let second = (
        harness.secondary_binding.queue.clone(),
        harness.secondary_binding.lane.clone(),
    );
    assert_eq!(seen, vec![first.clone(), second.clone(), first, second]);
}

pub async fn assert_route_fairness_jitter_stays_bounded<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };

    for _ in 0..8 {
        queue
            .enqueue(runtime_task(
                &harness.binding.queue,
                &harness.binding.lane,
                3,
                harness.retry_delay_ms,
            ))
            .await
            .expect("enqueue should succeed");
        queue
            .enqueue(runtime_task(
                &harness.secondary_binding.queue,
                &harness.secondary_binding.lane,
                3,
                harness.retry_delay_ms,
            ))
            .await
            .expect("enqueue should succeed");
    }

    let bindings = vec![harness.binding.clone(), harness.secondary_binding.clone()];
    let mut primary_count = 0usize;
    let mut secondary_count = 0usize;
    let mut running_delta = 0isize;

    for _ in 0..16 {
        let delivery = reserve_until(&queue, &bindings, Duration::from_secs(3))
            .await
            .expect("delivery should exist");
        if delivery.task.queue == harness.binding.queue
            && delivery.task.lane == harness.binding.lane
        {
            primary_count += 1;
            running_delta += 1;
        } else {
            secondary_count += 1;
            running_delta -= 1;
        }

        assert!(
            running_delta.abs() <= 2,
            "route fairness jitter drifted too far: {running_delta}"
        );
        queue
            .ack(&delivery.delivery_id)
            .await
            .expect("ack should succeed");
    }

    assert_eq!(primary_count, 8);
    assert_eq!(secondary_count, 8);
}

pub async fn assert_dead_letter_replay_is_consistent<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        1,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");
    let disposition = queue
        .retry(
            &delivery.delivery_id,
            "consistent failure",
            harness.retry_delay_ms,
        )
        .await
        .expect("retry should dead-letter");
    assert_eq!(format!("{disposition:?}"), "DeadLettered");

    let dead_letters = queue
        .dead_letters()
        .await
        .expect("dead letters should be readable");
    assert_eq!(dead_letters.len(), 1);
    let replay_delivery_id = dead_letters[0].delivery_id.clone();

    queue
        .replay_dead_letter(&replay_delivery_id)
        .await
        .expect("replay should succeed");

    let after_replay = queue
        .dead_letters()
        .await
        .expect("dead letters should be readable after replay");
    assert!(
        after_replay
            .iter()
            .all(|record| record.delivery_id != replay_delivery_id),
        "replayed dead-letter record should be removed from dead-letter store"
    );

    let replayed = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("replayed delivery should exist");
    assert_eq!(replayed.task.task_id, task.task_id);
    queue
        .ack(&replayed.delivery_id)
        .await
        .expect("ack should succeed");

    let duplicate = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_millis(300),
    )
    .await;
    assert!(
        duplicate.is_none(),
        "replayed task should not produce a duplicate delivery after ack"
    );
}

pub async fn assert_duplicate_delivery_is_eventually_singular<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    if !harness.capabilities.enhanced.crash_reclaim {
        return;
    }

    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        3,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");
    drop(queue);

    sleep(Duration::from_millis(harness.reclaim_wait_ms)).await;
    let reopened = harness.build_queue().await.expect("queue should rebuild");
    let redelivery = reserve_until(
        &reopened,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("redelivery should exist");
    assert_eq!(redelivery.task.task_id, delivery.task.task_id);
    reopened
        .ack(&redelivery.delivery_id)
        .await
        .expect("ack should succeed");

    let duplicate = reserve_until(
        &reopened,
        std::slice::from_ref(&harness.binding),
        Duration::from_millis(400),
    )
    .await;
    assert!(
        duplicate.is_none(),
        "after reclaim + ack, no duplicate delivery should remain visible"
    );
}

pub async fn assert_reclaims_unacked_delivery_after_reconnect<C>(harness: BrokerContractHarness<C>)
where
    C: Clone + Send + Sync + 'static,
{
    if !harness.capabilities.enhanced.crash_reclaim {
        return;
    }

    let Some(queue) = harness.build_queue().await else {
        return;
    };
    let task = runtime_task(
        &harness.binding.queue,
        &harness.binding.lane,
        3,
        harness.retry_delay_ms,
    );

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("delivery should exist");
    assert_eq!(delivery.task.task_id, task.task_id);

    drop(queue);
    sleep(Duration::from_millis(harness.reclaim_wait_ms)).await;

    let reopened = harness.build_queue().await.expect("queue should rebuild");
    let reclaimed = reserve_until(
        &reopened,
        std::slice::from_ref(&harness.binding),
        Duration::from_secs(3),
    )
    .await
    .expect("reclaimed delivery should exist");
    assert_eq!(reclaimed.task.task_id, task.task_id);
}

pub async fn reserve_until(
    queue: &Arc<dyn RuntimeTaskQueue>,
    bindings: &[RuntimeRouteBinding],
    timeout: Duration,
) -> Option<RuntimeTaskDelivery> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(delivery) = queue.reserve(bindings).await {
            return Some(delivery);
        }
        if Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn queue_stats_until(
    queue: &Arc<dyn RuntimeTaskQueue>,
    predicate: impl Fn(&[nexus_runtime::RuntimeQueueStats]) -> bool,
    timeout: Duration,
) -> Option<Vec<nexus_runtime::RuntimeQueueStats>> {
    let deadline = Instant::now() + timeout;
    loop {
        let stats = queue.stats().await.ok()?;
        if predicate(&stats) {
            return Some(stats);
        }
        if Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_millis(50)).await;
    }
}

fn stats_for<'a>(
    stats: &'a [nexus_runtime::RuntimeQueueStats],
    binding: &RuntimeRouteBinding,
) -> Option<&'a nexus_runtime::RuntimeQueueStats> {
    stats
        .iter()
        .find(|stat| stat.queue == binding.queue && stat.lane == binding.lane)
}

fn runtime_task(queue: &str, lane: &str, max_attempts: u32, retry_delay_ms: u64) -> RuntimeTask {
    let task_id = format!("task-{}", Ulid::new());
    RuntimeTask {
        task_id: task_id.clone(),
        task_type: RuntimeTaskType::OjJudge,
        source_domain: "oj".to_owned(),
        source_entity_id: format!("sub-{task_id}"),
        queue: queue.to_owned(),
        lane: lane.to_owned(),
        retry_policy: RuntimeRetryPolicy {
            max_attempts,
            retry_delay_ms,
        },
        payload: RuntimeTaskPayload::OjJudge(OjJudgeTask {
            submission_id: SubmissionId::from(format!("sub-{task_id}")),
            problem_id: ProblemId::from("p-it"),
            user_id: UserId::from("u-it"),
            language: "cpp".to_owned(),
            judge_mode: RuntimeJudgeMode::Acm,
            sandbox_kind: RuntimeSandboxKind::Nsjail,
            source_code: "int main() { return 0; }".to_owned(),
            limits: RuntimeLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
            testcases: vec![RuntimeTestcase {
                case_no: 1,
                input: "1\n".to_owned(),
                expected_output: "1\n".to_owned(),
                score: 100,
            }],
            judge_config: None,
        }),
    }
}
