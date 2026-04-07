use async_trait::async_trait;
use nexus_runtime::{RuntimeEventObserver, RuntimeTaskEvent};
use nexus_shared::AppResult;

use crate::{
    model::{JobEvent, JobEventMetadata, JobEventSource, JobStatus},
    registry::SharedJobEventStore,
};

pub struct JobRuntimeEventObserver {
    event_store: SharedJobEventStore,
}

impl JobRuntimeEventObserver {
    pub fn new(event_store: SharedJobEventStore) -> Self {
        Self { event_store }
    }
}

#[async_trait]
impl RuntimeEventObserver for JobRuntimeEventObserver {
    async fn on_event(&self, event: RuntimeTaskEvent) -> AppResult<()> {
        self.event_store.append(JobEvent {
            job_id: event.task_id.clone(),
            status: JobStatus::from(event.status),
            source: JobEventSource::Runtime,
            message: event.message,
            attempt: Some(event.attempt),
            occurred_at_ms: now_ms(),
            error: None,
            metadata: Some(JobEventMetadata {
                queue: Some(event.queue),
                lane: Some(event.lane),
                source_domain: Some(event.source_domain),
                source_entity_id: None,
                handler: None,
                execution_contract: None,
            }),
        });
        Ok(())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_runtime::{RuntimeEventObserver, RuntimeTaskEvent, RuntimeTaskLifecycleStatus};

    use crate::{
        registry::{InMemoryJobEventStore, JobEventStore},
        runtime::JobRuntimeEventObserver,
    };

    #[tokio::test]
    async fn runtime_observer_appends_job_event_history() {
        let store = Arc::new(InMemoryJobEventStore::default());
        let observer = JobRuntimeEventObserver::new(store.clone());

        observer
            .on_event(RuntimeTaskEvent {
                task_id: "job-1".to_owned(),
                source_domain: "oj".to_owned(),
                queue: "oj_judge".to_owned(),
                lane: "fast".to_owned(),
                attempt: 2,
                submission_id: Some("sub-1".to_owned()),
                problem_id: Some("prob-1".to_owned()),
                user_id: Some("user-1".to_owned()),
                language: Some("rust".to_owned()),
                status: RuntimeTaskLifecycleStatus::Running,
                message: "runtime task is running".to_owned(),
                execution_id: Some("exec-1".to_owned()),
                outcome: None,
            })
            .await
            .expect("observer should append event");

        let events = store.list("job-1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].job_id, "job-1");
        assert!(matches!(events[0].status, crate::model::JobStatus::Running));
    }
}
