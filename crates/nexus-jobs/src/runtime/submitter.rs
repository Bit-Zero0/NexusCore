use std::sync::Arc;

use async_trait::async_trait;
use nexus_runtime::RuntimeTaskService;
use nexus_shared::{AppError, AppResult};

use crate::{
    api::JobSubmitter,
    handlers::{
        JobExecutionContext, JobHandlerFailure, JobHandlerResult, SharedJobHandlerRegistry,
    },
    model::{JobDefinition, JobReceipt},
};

pub struct RuntimeBackedJobSubmitter {
    runtime_service: Arc<RuntimeTaskService>,
    handler_registry: SharedJobHandlerRegistry,
}

impl RuntimeBackedJobSubmitter {
    pub fn new(
        runtime_service: Arc<RuntimeTaskService>,
        handler_registry: SharedJobHandlerRegistry,
    ) -> Self {
        Self {
            runtime_service,
            handler_registry,
        }
    }
}

#[async_trait]
impl JobSubmitter for RuntimeBackedJobSubmitter {
    async fn submit(&self, job: JobDefinition) -> AppResult<JobReceipt> {
        let route = job.dispatch.route.clone();
        let job_id = job.job_id.0.clone();
        let handler = self
            .handler_registry
            .resolve_handler(
                job.job_type.namespace.as_str(),
                job.job_type.name.as_str(),
                job.job_type.version,
            )
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "no executable job handler for {}:{}:v{}",
                    job.job_type.namespace, job.job_type.name, job.job_type.version
                ))
            })?;
        let descriptor = handler.descriptor();
        let dispatch_result = handler
            .prepare_dispatch(&job, &JobExecutionContext::from_job(&job, now_ms()))
            .await?;
        let runtime_task = match dispatch_result {
            JobHandlerResult::Dispatch(plan) => plan.runtime_task,
            JobHandlerResult::Rejected(failure) => {
                return Err(map_handler_failure(&descriptor.key(), failure));
            }
        };
        self.runtime_service.schedule(runtime_task).await?;
        Ok(JobReceipt {
            job_id,
            queue: route.queue,
            lane: route.lane,
            handler: Some(descriptor.key()),
            execution_contract: Some(format_execution_contract(&descriptor.execution_contract)),
        })
    }
}

fn map_handler_failure(handler_key: &str, failure: JobHandlerFailure) -> AppError {
    let message = format!(
        "job handler {handler_key} rejected dispatch [{}]: {}",
        failure.code, failure.reason
    );
    if failure.retryable {
        AppError::InvalidConfig(message)
    } else {
        AppError::BadRequest(message)
    }
}

fn format_execution_contract(contract: &crate::handlers::JobExecutionContract) -> String {
    match contract {
        crate::handlers::JobExecutionContract::RuntimeTask => "runtime_task".to_owned(),
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
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use nexus_runtime::{
        build_default_runtime_catalog, InMemoryRuntimeTaskQueue, NoopRuntimeEventObserver,
        RuntimeJudgeMode, RuntimeLimits, RuntimeSandboxKind, RuntimeSeccompMode,
        RuntimeSyscallArch, RuntimeSyscallFlavor, RuntimeTaskService, RuntimeWorker,
    };
    use nexus_shared::{AppError, AppResult, ProblemId, SubmissionId, UserId};

    use crate::{
        api::JobSubmitter,
        domains::{build_oj_judge_job, oj_judge_handler_descriptor, OjJudgeJobInput},
        handlers::{
            InMemoryJobHandlerRegistry, JobDispatchPlan, JobExecutionContext, JobHandler,
            JobHandlerDescriptor, JobHandlerFailure, JobHandlerRegistry, JobHandlerResult,
        },
        model::{JobDefinition, JobRetryPolicy, JobRoute},
        runtime::RuntimeBackedJobSubmitter,
    };

    struct RecordingOjHandler {
        last_context: Arc<Mutex<Option<JobExecutionContext>>>,
    }

    #[async_trait]
    impl JobHandler for RecordingOjHandler {
        fn descriptor(&self) -> JobHandlerDescriptor {
            oj_judge_handler_descriptor()
        }

        async fn prepare_dispatch(
            &self,
            job: &JobDefinition,
            context: &JobExecutionContext,
        ) -> AppResult<JobHandlerResult> {
            *self.last_context.lock().expect("context mutex poisoned") = Some(context.clone());
            Ok(JobHandlerResult::Dispatch(JobDispatchPlan::runtime_task(
                crate::runtime::map_job_to_runtime_task(job)?,
                self.descriptor(),
            )))
        }
    }

    #[tokio::test]
    async fn runtime_submitter_builds_handler_context_and_receipt_metadata() {
        let runtime_service = Arc::new(RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                build_default_runtime_catalog(),
                "/tmp/nexus-jobs-submitter-test",
                "/usr/bin/nsjail",
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                RuntimeSyscallArch::X86_64,
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(NoopRuntimeEventObserver),
        ));
        let registry = Arc::new(InMemoryJobHandlerRegistry::default());
        let last_context = Arc::new(Mutex::new(None));
        registry.register_handler(Arc::new(RecordingOjHandler {
            last_context: last_context.clone(),
        }));
        let submitter = RuntimeBackedJobSubmitter::new(runtime_service, registry);

        let receipt = submitter
            .submit(build_oj_job())
            .await
            .expect("job should submit");

        let context = last_context
            .lock()
            .expect("context mutex poisoned")
            .clone()
            .expect("handler should have recorded context");
        assert_eq!(context.job_id, "job-1");
        assert_eq!(context.namespace, "oj");
        assert_eq!(context.route.queue, "oj_judge");
        assert_eq!(context.route.lane, "fast");
        assert_eq!(context.origin.source_domain, "oj");
        assert_eq!(receipt.handler.as_deref(), Some("oj:judge_submission:v1"));
        assert_eq!(receipt.execution_contract.as_deref(), Some("runtime_task"));
    }

    struct RejectingOjHandler;

    #[async_trait]
    impl JobHandler for RejectingOjHandler {
        fn descriptor(&self) -> JobHandlerDescriptor {
            oj_judge_handler_descriptor()
        }

        async fn prepare_dispatch(
            &self,
            _job: &JobDefinition,
            _context: &JobExecutionContext,
        ) -> AppResult<JobHandlerResult> {
            Ok(JobHandlerResult::Rejected(JobHandlerFailure::rejected(
                "payload_invalid",
                "judge payload missing required testcase assets",
            )))
        }
    }

    #[tokio::test]
    async fn runtime_submitter_surfaces_structured_handler_rejection() {
        let runtime_service = Arc::new(RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                build_default_runtime_catalog(),
                "/tmp/nexus-jobs-submitter-test-reject",
                "/usr/bin/nsjail",
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                RuntimeSyscallArch::X86_64,
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(NoopRuntimeEventObserver),
        ));
        let registry = Arc::new(InMemoryJobHandlerRegistry::default());
        registry.register_handler(Arc::new(RejectingOjHandler));
        let submitter = RuntimeBackedJobSubmitter::new(runtime_service, registry);

        let error = submitter
            .submit(build_oj_job())
            .await
            .expect_err("job should be rejected");

        match error {
            AppError::BadRequest(message) => {
                assert!(message.contains("payload_invalid"));
                assert!(message.contains("judge payload missing required testcase assets"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    fn build_oj_job() -> JobDefinition {
        build_oj_judge_job(OjJudgeJobInput {
            job_id: "job-1".to_owned(),
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
        })
    }
}
