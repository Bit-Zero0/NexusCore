use std::sync::Arc;

use async_trait::async_trait;
use nexus_runtime::RuntimeTaskService;
use nexus_shared::{AppError, AppResult};
use serde::Deserialize;

use crate::{
    handlers::SharedJobHandlerRegistry,
    model::{
        JobDefinition, JobEvent, JobManagementSummary, JobManagementView, JobSnapshot, JobStatus,
        JobSubmissionFailureSnapshot,
    },
    registry::{SharedJobDefinitionStore, SharedJobEventStore},
};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct JobQueryFilter {
    pub namespace: Option<String>,
    pub source_domain: Option<String>,
    pub queue: Option<String>,
    pub lane: Option<String>,
    pub status: Option<JobStatus>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[async_trait]
pub trait JobQueryService: Send + Sync {
    async fn get_job(&self, job_id: &str) -> AppResult<JobSnapshot>;
    async fn get_job_history(&self, job_id: &str) -> AppResult<Vec<JobEvent>>;
    async fn list_jobs(&self, filter: &JobQueryFilter) -> AppResult<Vec<JobSnapshot>>;
    async fn management_view(&self, filter: &JobQueryFilter) -> AppResult<JobManagementView>;
}

pub struct DefaultJobQueryService {
    runtime_service: Arc<RuntimeTaskService>,
    definition_store: SharedJobDefinitionStore,
    event_store: SharedJobEventStore,
    handler_registry: SharedJobHandlerRegistry,
}

impl DefaultJobQueryService {
    pub fn new(
        runtime_service: Arc<RuntimeTaskService>,
        definition_store: SharedJobDefinitionStore,
        event_store: SharedJobEventStore,
        handler_registry: SharedJobHandlerRegistry,
    ) -> Self {
        Self {
            runtime_service,
            definition_store,
            event_store,
            handler_registry,
        }
    }
}

#[async_trait]
impl JobQueryService for DefaultJobQueryService {
    async fn get_job(&self, job_id: &str) -> AppResult<JobSnapshot> {
        let definition = self
            .definition_store
            .get(job_id)
            .ok_or_else(|| AppError::NotFound(format!("job not found: {job_id}")))?;
        let history = self.event_store.list(job_id);
        match self.runtime_service.get_task(job_id) {
            Ok(snapshot) => Ok(build_job_snapshot(
                definition,
                snapshot,
                history,
                self.handler_registry.clone(),
            )),
            Err(AppError::NotFound(_)) => build_job_snapshot_without_runtime(
                definition,
                history,
                self.handler_registry.clone(),
            )
            .ok_or_else(|| AppError::NotFound(format!("job not found: {job_id}"))),
            Err(error) => Err(error),
        }
    }

    async fn list_jobs(&self, filter: &JobQueryFilter) -> AppResult<Vec<JobSnapshot>> {
        let mut jobs = collect_matching_jobs(
            &self.runtime_service,
            &self.definition_store,
            &self.event_store,
            &self.handler_registry,
            filter,
        )?;
        jobs.sort_by(|left, right| left.job_id.cmp(&right.job_id));
        Ok(paginate(jobs, filter))
    }

    async fn get_job_history(&self, job_id: &str) -> AppResult<Vec<JobEvent>> {
        self.definition_store
            .get(job_id)
            .ok_or_else(|| AppError::NotFound(format!("job not found: {job_id}")))?;
        Ok(self.event_store.list(job_id))
    }

    async fn management_view(&self, filter: &JobQueryFilter) -> AppResult<JobManagementView> {
        let all_jobs = collect_matching_jobs(
            &self.runtime_service,
            &self.definition_store,
            &self.event_store,
            &self.handler_registry,
            filter,
        )?;
        let jobs = paginate(all_jobs.clone(), filter);
        let mut summary = JobManagementSummary {
            total_jobs: all_jobs.len(),
            ..Default::default()
        };
        for job in &all_jobs {
            match job.status {
                JobStatus::Created | JobStatus::Enqueued | JobStatus::Leased => summary.queued += 1,
                JobStatus::Running => summary.running += 1,
                JobStatus::Succeeded => summary.succeeded += 1,
                JobStatus::Retrying => summary.retrying += 1,
                JobStatus::DeadLettered => summary.dead_lettered += 1,
                JobStatus::Failed | JobStatus::Cancelled => {
                    summary.failed += 1;
                    if job.latest_submission_failure.is_some() {
                        summary.submission_rejected += 1;
                    }
                }
            }
        }
        Ok(JobManagementView { jobs, summary })
    }
}

fn collect_matching_jobs(
    runtime_service: &Arc<RuntimeTaskService>,
    definition_store: &SharedJobDefinitionStore,
    event_store: &SharedJobEventStore,
    handler_registry: &SharedJobHandlerRegistry,
    filter: &JobQueryFilter,
) -> AppResult<Vec<JobSnapshot>> {
    let mut jobs = Vec::new();
    for definition in definition_store.list() {
        let history = event_store.list(&definition.job_id.0);
        let snapshot = match runtime_service.get_task(&definition.job_id.0) {
            Ok(runtime_snapshot) => build_job_snapshot(
                definition.clone(),
                runtime_snapshot,
                history,
                handler_registry.clone(),
            ),
            Err(AppError::NotFound(_)) => {
                let Some(snapshot) = build_job_snapshot_without_runtime(
                    definition.clone(),
                    history,
                    handler_registry.clone(),
                ) else {
                    continue;
                };
                snapshot
            }
            Err(error) => return Err(error),
        };
        if matches_job_filter(&snapshot, filter) {
            jobs.push(snapshot);
        }
    }
    Ok(jobs)
}

fn build_job_snapshot(
    definition: JobDefinition,
    runtime_snapshot: nexus_runtime::RuntimeTaskSnapshot,
    history: Vec<JobEvent>,
    handler_registry: SharedJobHandlerRegistry,
) -> JobSnapshot {
    let latest_submission_failure = latest_submission_failure(&history);
    JobSnapshot {
        job_id: definition.job_id.0.clone(),
        job_type: definition.job_type.clone(),
        origin: definition.origin.clone(),
        dispatch: definition.dispatch.clone(),
        status: runtime_snapshot.status.into(),
        message: runtime_snapshot.message,
        error: runtime_snapshot.error,
        handler: handler_registry.resolve_descriptor(
            definition.job_type.namespace.as_str(),
            definition.job_type.name.as_str(),
            definition.job_type.version,
        ),
        latest_submission_failure,
        history,
    }
}

fn build_job_snapshot_without_runtime(
    definition: JobDefinition,
    history: Vec<JobEvent>,
    handler_registry: SharedJobHandlerRegistry,
) -> Option<JobSnapshot> {
    let latest_event = history.last()?.clone();
    let latest_submission_failure = latest_submission_failure(&history);
    Some(JobSnapshot {
        job_id: definition.job_id.0.clone(),
        job_type: definition.job_type.clone(),
        origin: definition.origin.clone(),
        dispatch: definition.dispatch.clone(),
        status: latest_event.status,
        message: latest_event.message,
        error: latest_event.error,
        handler: handler_registry.resolve_descriptor(
            definition.job_type.namespace.as_str(),
            definition.job_type.name.as_str(),
            definition.job_type.version,
        ),
        latest_submission_failure,
        history,
    })
}

fn latest_submission_failure(history: &[JobEvent]) -> Option<JobSubmissionFailureSnapshot> {
    history
        .iter()
        .rev()
        .find(|event| {
            matches!(event.source, crate::model::JobEventSource::JobPlatform)
                && matches!(event.status, JobStatus::Failed)
                && event.error.is_some()
        })
        .map(|event| JobSubmissionFailureSnapshot {
            message: event.error.clone().unwrap_or_else(|| event.message.clone()),
            occurred_at_ms: event.occurred_at_ms,
        })
}

fn matches_job_filter(snapshot: &JobSnapshot, filter: &JobQueryFilter) -> bool {
    filter
        .namespace
        .as_deref()
        .map_or(true, |value| snapshot.job_type.namespace == value)
        && filter
            .source_domain
            .as_deref()
            .map_or(true, |value| snapshot.origin.source_domain == value)
        && filter
            .queue
            .as_deref()
            .map_or(true, |value| snapshot.dispatch.route.queue == value)
        && filter
            .lane
            .as_deref()
            .map_or(true, |value| snapshot.dispatch.route.lane == value)
        && filter
            .status
            .as_ref()
            .map_or(true, |value| &snapshot.status == value)
}

fn paginate<T>(items: Vec<T>, filter: &JobQueryFilter) -> Vec<T> {
    let offset = filter.offset.unwrap_or(0);
    let limit = filter.limit.unwrap_or(50);
    items.into_iter().skip(offset).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_runtime::{
        build_default_runtime_catalog, InMemoryRuntimeTaskQueue, NoopRuntimeEventObserver,
        RuntimeJudgeMode, RuntimeLimits, RuntimeSandboxKind, RuntimeSeccompMode,
        RuntimeSyscallArch, RuntimeSyscallFlavor, RuntimeTaskService, RuntimeWorker,
    };
    use nexus_shared::{ProblemId, SubmissionId, UserId};

    use crate::{
        api::{DefaultJobQueryService, JobQueryFilter, JobQueryService},
        domains::{build_oj_judge_job, OjJudgeJobInput},
        handlers::{InMemoryJobHandlerRegistry, JobHandlerRegistry},
        model::{JobEvent, JobRetryPolicy, JobRoute, JobStatus},
        registry::{
            InMemoryJobDefinitionStore, InMemoryJobEventStore, JobDefinitionStore, JobEventStore,
        },
        runtime::map_job_to_runtime_task,
    };

    #[tokio::test]
    async fn query_service_returns_job_snapshot_and_summary() {
        let runtime_service = Arc::new(RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                build_default_runtime_catalog(),
                "/tmp/nexus-jobs-query-test",
                "/usr/bin/nsjail",
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                RuntimeSyscallArch::X86_64,
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(NoopRuntimeEventObserver),
        ));
        let store = Arc::new(InMemoryJobDefinitionStore::default());
        let event_store = Arc::new(InMemoryJobEventStore::default());
        let handler_registry = Arc::new(InMemoryJobHandlerRegistry::default());
        handler_registry.register_descriptor(crate::domains::oj_judge_handler_descriptor());
        let query_service = DefaultJobQueryService::new(
            runtime_service.clone(),
            store.clone(),
            event_store,
            handler_registry,
        );

        let job = build_oj_judge_job(OjJudgeJobInput {
            job_id: "task-sub-1".to_owned(),
            source_entity_id: "sub-1".to_owned(),
            submission_id: SubmissionId("sub-1".to_owned()),
            problem_id: ProblemId("prob-1".to_owned()),
            user_id: UserId("user-1".to_owned()),
            language: "rust".to_owned(),
            judge_mode: RuntimeJudgeMode::Acm,
            sandbox_kind: RuntimeSandboxKind::Nsjail,
            source_code: "fn main() {}".to_owned(),
            limits: RuntimeLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
            testcases: Vec::new(),
            judge_config: None,
            route: JobRoute {
                queue: "oj_judge".to_owned(),
                lane: "fast".to_owned(),
            },
            retry_policy: JobRetryPolicy {
                max_attempts: 3,
                retry_delay_ms: 1000,
            },
        });
        store.save(job.clone());
        runtime_service
            .schedule(map_job_to_runtime_task(&job).expect("job should map"))
            .await
            .expect("job should schedule");

        let snapshot = query_service
            .get_job("task-sub-1")
            .await
            .expect("job snapshot should exist");
        assert_eq!(snapshot.job_type.namespace, "oj");
        assert!(matches!(snapshot.status, JobStatus::Enqueued));

        let management = query_service
            .management_view(&JobQueryFilter {
                namespace: Some("oj".to_owned()),
                ..Default::default()
            })
            .await
            .expect("management view should exist");
        assert_eq!(management.summary.total_jobs, 1);
        assert_eq!(management.summary.queued, 1);
        assert_eq!(management.jobs.len(), 1);
        assert!(query_service
            .get_job_history("task-sub-1")
            .await
            .expect("history should exist")
            .is_empty());
    }

    #[tokio::test]
    async fn query_service_surfaces_submission_rejection_without_runtime_snapshot() {
        let runtime_service = Arc::new(RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                build_default_runtime_catalog(),
                "/tmp/nexus-jobs-query-rejection-test",
                "/usr/bin/nsjail",
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                RuntimeSyscallArch::X86_64,
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(NoopRuntimeEventObserver),
        ));
        let store = Arc::new(InMemoryJobDefinitionStore::default());
        let event_store = Arc::new(InMemoryJobEventStore::default());
        let handler_registry = Arc::new(InMemoryJobHandlerRegistry::default());
        handler_registry.register_descriptor(crate::domains::oj_judge_handler_descriptor());
        let query_service = DefaultJobQueryService::new(
            runtime_service,
            store.clone(),
            event_store.clone(),
            handler_registry,
        );
        let job = build_oj_judge_job(OjJudgeJobInput {
            job_id: "job-rejected-1".to_owned(),
            source_entity_id: "sub-rejected-1".to_owned(),
            submission_id: SubmissionId("sub-rejected-1".to_owned()),
            problem_id: ProblemId("prob-1".to_owned()),
            user_id: UserId("user-1".to_owned()),
            language: "rust".to_owned(),
            judge_mode: RuntimeJudgeMode::Acm,
            sandbox_kind: RuntimeSandboxKind::Nsjail,
            source_code: "fn main() {}".to_owned(),
            limits: RuntimeLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
            testcases: Vec::new(),
            judge_config: None,
            route: JobRoute {
                queue: "oj_judge".to_owned(),
                lane: "fast".to_owned(),
            },
            retry_policy: JobRetryPolicy {
                max_attempts: 3,
                retry_delay_ms: 1000,
            },
        });
        store.save(job);
        event_store.append(JobEvent {
            job_id: "job-rejected-1".to_owned(),
            status: JobStatus::Failed,
            source: crate::model::JobEventSource::JobPlatform,
            message: "job submission rejected before runtime dispatch".to_owned(),
            attempt: Some(1),
            occurred_at_ms: 42,
            error: Some("handler rejected dispatch".to_owned()),
            metadata: None,
        });

        let snapshot = query_service
            .get_job("job-rejected-1")
            .await
            .expect("rejected job snapshot should exist");
        assert!(matches!(snapshot.status, JobStatus::Failed));
        assert_eq!(
            snapshot
                .latest_submission_failure
                .expect("submission failure should exist")
                .message,
            "handler rejected dispatch"
        );
        assert!(snapshot.handler.is_some());
    }
}
