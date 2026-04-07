use nexus_runtime::{
    OjJudgeTask, RuntimeRetryPolicy, RuntimeTask, RuntimeTaskPayload, RuntimeTaskType,
};
use nexus_shared::{AppError, AppResult};

use crate::model::{JobDefinition, JobPayload};

pub fn map_job_to_runtime_task(job: &JobDefinition) -> AppResult<RuntimeTask> {
    let payload = match &job.payload {
        JobPayload::OjJudge(payload) => RuntimeTaskPayload::OjJudge(OjJudgeTask {
            submission_id: payload.submission_id.clone(),
            problem_id: payload.problem_id.clone(),
            user_id: payload.user_id.clone(),
            language: payload.language.clone(),
            judge_mode: payload.judge_mode.clone(),
            sandbox_kind: payload.sandbox_kind,
            source_code: payload.source_code.clone(),
            limits: payload.limits.clone(),
            testcases: payload.testcases.clone(),
            judge_config: payload.judge_config.clone(),
        }),
        JobPayload::Json(payload) => {
            return Err(AppError::BadRequest(format!(
                "job payload schema {} is not yet mappable to runtime tasks",
                payload.schema
            )));
        }
    };

    let task_type = match &job.payload {
        JobPayload::OjJudge(_) => RuntimeTaskType::OjJudge,
        JobPayload::Json(_) => {
            return Err(AppError::BadRequest(
                "json jobs are not yet executable by runtime".to_owned(),
            ));
        }
    };

    Ok(RuntimeTask {
        task_id: job.job_id.0.clone(),
        task_type,
        source_domain: job.origin.source_domain.clone(),
        source_entity_id: job.origin.source_entity_id.clone(),
        queue: job.dispatch.route.queue.clone(),
        lane: job.dispatch.route.lane.clone(),
        retry_policy: RuntimeRetryPolicy {
            max_attempts: job.dispatch.retry_policy.max_attempts,
            retry_delay_ms: job.dispatch.retry_policy.retry_delay_ms,
        },
        payload,
    })
}

#[cfg(test)]
mod tests {
    use nexus_runtime::{RuntimeJudgeMode, RuntimeLimits, RuntimeSandboxKind, RuntimeTaskPayload};
    use nexus_shared::{ProblemId, SubmissionId, UserId};

    use crate::{
        domains::{build_oj_judge_job, OjJudgeJobInput},
        model::{JobRetryPolicy, JobRoute},
        runtime::map_job_to_runtime_task,
    };

    #[test]
    fn oj_job_maps_to_runtime_task() {
        let job = build_oj_judge_job(OjJudgeJobInput {
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
        });

        let task = map_job_to_runtime_task(&job).expect("oj job should map");
        assert_eq!(task.task_id, "job-1");
        assert_eq!(task.queue, "oj_judge");
        assert_eq!(task.lane, "fast");
        assert_eq!(task.retry_policy.max_attempts, 3);
        assert_eq!(task.retry_policy.retry_delay_ms, 1000);
        assert!(matches!(task.payload, RuntimeTaskPayload::OjJudge(_)));
    }
}
