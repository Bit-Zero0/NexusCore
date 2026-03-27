use std::sync::Arc;

use nexus_runtime::{
    build_rabbitmq_runtime_queue, OjJudgeTask, RabbitMqQueueConfig, RuntimeJudgeMode,
    RuntimeLimits, RuntimeRetryPolicy, RuntimeRouteBinding, RuntimeSandboxKind, RuntimeTask,
    RuntimeTaskPayload, RuntimeTaskQueue, RuntimeTaskType, RuntimeTestcase,
};
use nexus_shared::{ProblemId, SubmissionId, UserId};
use tokio::time::{sleep, Duration, Instant};
use ulid::Ulid;

#[tokio::test]
async fn rabbitmq_publish_reserve_ack_roundtrip() {
    let Some((queue, binding)) = test_queue().await else {
        return;
    };
    let task = runtime_task(&binding.queue, &binding.lane, 3);

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");
    let delivery = reserve_until(
        &queue,
        std::slice::from_ref(&binding),
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

#[tokio::test]
async fn rabbitmq_retry_dead_letter_and_replay_roundtrip() {
    let Some((queue, binding)) = test_queue().await else {
        return;
    };
    let mut task = runtime_task(&binding.queue, &binding.lane, 2);
    task.retry_policy.retry_delay_ms = 50;

    queue
        .enqueue(task.clone())
        .await
        .expect("enqueue should succeed");

    let first = reserve_until(
        &queue,
        std::slice::from_ref(&binding),
        Duration::from_secs(3),
    )
    .await
    .expect("first delivery should exist");
    let disposition = queue
        .retry(&first.delivery_id, "transient failure", 50)
        .await
        .expect("retry should succeed");
    assert_eq!(format!("{disposition:?}"), "Requeued");

    let second = reserve_until(
        &queue,
        std::slice::from_ref(&binding),
        Duration::from_secs(5),
    )
    .await
    .expect("second delivery should exist");
    assert_eq!(second.attempt, 2);
    assert_eq!(second.last_error.as_deref(), Some("transient failure"));

    let disposition = queue
        .retry(&second.delivery_id, "persistent failure", 50)
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
        std::slice::from_ref(&binding),
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

async fn test_queue() -> Option<(Arc<dyn RuntimeTaskQueue>, RuntimeRouteBinding)> {
    let url = std::env::var("NEXUS_RABBITMQ_TEST_URL").ok()?;
    let suffix = Ulid::new().to_string().to_lowercase();
    let binding = RuntimeRouteBinding {
        queue: format!("it_queue_{suffix}"),
        lane: "fast".to_owned(),
    };
    let queue = build_rabbitmq_runtime_queue(RabbitMqQueueConfig {
        url,
        exchange: format!("nexus.runtime.it.{suffix}"),
        queue_prefix: format!("nexus.runtime.it.{suffix}"),
    })
    .await
    .ok()?;

    Some((queue, binding))
}

async fn reserve_until(
    queue: &Arc<dyn RuntimeTaskQueue>,
    bindings: &[RuntimeRouteBinding],
    timeout: Duration,
) -> Option<nexus_runtime::RuntimeTaskDelivery> {
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

fn runtime_task(queue: &str, lane: &str, max_attempts: u32) -> RuntimeTask {
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
            retry_delay_ms: 50,
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
