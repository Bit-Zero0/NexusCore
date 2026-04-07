use nexus_shared::{AppError, AppResult};

use crate::{
    api::JobSubmissionValidator,
    handlers::SharedJobHandlerRegistry,
    model::{JobDefinition, JobPayload},
};

pub struct DefaultJobSubmissionValidator {
    handler_registry: SharedJobHandlerRegistry,
}

impl DefaultJobSubmissionValidator {
    pub fn new(handler_registry: SharedJobHandlerRegistry) -> Self {
        Self { handler_registry }
    }
}

impl JobSubmissionValidator for DefaultJobSubmissionValidator {
    fn validate(&self, job: &JobDefinition) -> AppResult<()> {
        if job.job_id.0.trim().is_empty() {
            return Err(AppError::BadRequest("job_id must not be empty".to_owned()));
        }
        if job.job_type.namespace.trim().is_empty() || job.job_type.name.trim().is_empty() {
            return Err(AppError::BadRequest(
                "job_type namespace/name must not be empty".to_owned(),
            ));
        }
        if job.namespace.0.trim().is_empty() {
            return Err(AppError::BadRequest(
                "job namespace must not be empty".to_owned(),
            ));
        }
        if job.origin.source_domain.trim().is_empty()
            || job.origin.source_entity_id.trim().is_empty()
        {
            return Err(AppError::BadRequest(
                "job origin must include source_domain and source_entity_id".to_owned(),
            ));
        }
        if job.dispatch.route.queue.trim().is_empty() || job.dispatch.route.lane.trim().is_empty() {
            return Err(AppError::BadRequest(
                "job route queue/lane must not be empty".to_owned(),
            ));
        }
        if job.dispatch.retry_policy.max_attempts == 0 {
            return Err(AppError::BadRequest(
                "job retry policy max_attempts must be greater than zero".to_owned(),
            ));
        }
        if self
            .handler_registry
            .resolve_descriptor(
                job.job_type.namespace.as_str(),
                job.job_type.name.as_str(),
                job.job_type.version,
            )
            .is_none()
        {
            return Err(AppError::BadRequest(format!(
                "no registered job handler for {}:{}:v{}",
                job.job_type.namespace, job.job_type.name, job.job_type.version
            )));
        }

        match &job.payload {
            JobPayload::OjJudge(payload) => {
                if payload.source_code.trim().is_empty() {
                    return Err(AppError::BadRequest(
                        "oj job source_code must not be empty".to_owned(),
                    ));
                }
            }
            JobPayload::Json(payload) => {
                if payload.schema.trim().is_empty() {
                    return Err(AppError::BadRequest(
                        "json job schema must not be empty".to_owned(),
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_runtime::{RuntimeJudgeMode, RuntimeLimits, RuntimeSandboxKind};
    use nexus_shared::{ProblemId, SubmissionId, UserId};

    use crate::{
        api::{DefaultJobSubmissionValidator, JobSubmissionValidator},
        domains::{build_oj_judge_job, oj_judge_job_handler, OjJudgeJobInput},
        handlers::{InMemoryJobHandlerRegistry, JobHandlerRegistry},
        model::{JobRetryPolicy, JobRoute},
    };

    #[test]
    fn validator_rejects_job_without_registered_handler() {
        let registry = Arc::new(InMemoryJobHandlerRegistry::default());
        let validator = DefaultJobSubmissionValidator::new(registry);
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

        assert!(validator.validate(&job).is_err());
    }

    #[test]
    fn validator_accepts_job_with_registered_handler() {
        let registry = Arc::new(InMemoryJobHandlerRegistry::default());
        registry.register_handler(oj_judge_job_handler());
        let validator = DefaultJobSubmissionValidator::new(registry);
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

        validator
            .validate(&job)
            .expect("registered handler should allow submission");
    }
}
