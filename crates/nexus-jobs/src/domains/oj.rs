use std::sync::Arc;

use async_trait::async_trait;
use nexus_runtime::{
    RuntimeFunctionSignature, RuntimeJudgeConfig, RuntimeJudgeMode, RuntimeLimits,
    RuntimeSandboxKind, RuntimeSpjConfig, RuntimeTestcase, RuntimeValidatorConfig,
};
use nexus_shared::{AppError, AppResult, ProblemId, SubmissionId, UserId};

use crate::handlers::{
    JobDispatchPlan, JobExecutionContext, JobExecutionContract, JobHandler, JobHandlerCapabilities,
    JobHandlerDescriptor, JobHandlerResult,
};
use crate::model::{
    JobDefinition, JobDispatch, JobNamespace, JobOrigin, JobPayload, JobRetryPolicy, JobRoute,
    JobTimeoutPolicy, JobType,
};
use crate::runtime::map_job_to_runtime_task;

pub const OJ_JOB_NAMESPACE: &str = "oj";
pub const OJ_JOB_SOURCE_DOMAIN: &str = "oj";
pub const OJ_JUDGE_JOB_NAME: &str = "judge_submission";

pub fn oj_judge_handler_descriptor() -> JobHandlerDescriptor {
    JobHandlerDescriptor {
        job_type: JobType {
            namespace: OJ_JOB_NAMESPACE.into(),
            name: OJ_JUDGE_JOB_NAME.to_owned(),
            version: 1,
        },
        description: "OJ submission judge job executed by runtime task worker".to_owned(),
        execution_contract: JobExecutionContract::RuntimeTask,
        supports_replay: true,
        idempotent_submission: false,
        capabilities: JobHandlerCapabilities {
            validates_payload: true,
            requires_runtime_worker: true,
            supports_dry_run: false,
            emits_result_payload: false,
        },
    }
}

pub fn oj_judge_job_handler() -> Arc<dyn JobHandler> {
    Arc::new(OjJudgeRuntimeTaskHandler)
}

struct OjJudgeRuntimeTaskHandler;

#[async_trait]
impl JobHandler for OjJudgeRuntimeTaskHandler {
    fn descriptor(&self) -> JobHandlerDescriptor {
        oj_judge_handler_descriptor()
    }

    async fn prepare_dispatch(
        &self,
        job: &JobDefinition,
        _context: &JobExecutionContext,
    ) -> AppResult<JobHandlerResult> {
        if !matches!(job.payload, JobPayload::OjJudge(_)) {
            return Err(AppError::BadRequest(
                "oj judge handler can only dispatch oj_judge payloads".to_owned(),
            ));
        }
        Ok(JobHandlerResult::Dispatch(JobDispatchPlan::runtime_task(
            map_job_to_runtime_task(job)?,
            self.descriptor(),
        )))
    }
}

#[derive(Debug, Clone)]
pub struct OjJudgeJobInput {
    pub job_id: String,
    pub source_entity_id: String,
    pub submission_id: SubmissionId,
    pub problem_id: ProblemId,
    pub user_id: UserId,
    pub language: String,
    pub judge_mode: RuntimeJudgeMode,
    pub sandbox_kind: RuntimeSandboxKind,
    pub source_code: String,
    pub limits: RuntimeLimits,
    pub testcases: Vec<RuntimeTestcase>,
    pub judge_config: Option<RuntimeJudgeConfig>,
    pub route: JobRoute,
    pub retry_policy: JobRetryPolicy,
}

pub fn build_oj_judge_job(input: OjJudgeJobInput) -> JobDefinition {
    JobDefinition {
        job_id: input.job_id.into(),
        job_type: JobType {
            namespace: OJ_JOB_NAMESPACE.into(),
            name: OJ_JUDGE_JOB_NAME.to_owned(),
            version: 1,
        },
        namespace: JobNamespace::new(OJ_JOB_NAMESPACE),
        origin: JobOrigin {
            source_domain: OJ_JOB_SOURCE_DOMAIN.to_owned(),
            source_entity_id: input.source_entity_id,
            submitted_by: Some(input.user_id.0.clone()),
        },
        dispatch: JobDispatch {
            route: input.route,
            retry_policy: input.retry_policy,
            timeout_policy: JobTimeoutPolicy::default(),
        },
        payload: JobPayload::OjJudge(OjJudgeJobPayload {
            submission_id: input.submission_id,
            problem_id: input.problem_id,
            user_id: input.user_id,
            language: input.language,
            judge_mode: input.judge_mode,
            sandbox_kind: input.sandbox_kind,
            source_code: input.source_code,
            limits: input.limits,
            testcases: input.testcases,
            judge_config: input.judge_config,
        }),
        labels: Default::default(),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OjJudgeJobPayload {
    pub submission_id: SubmissionId,
    pub problem_id: ProblemId,
    pub user_id: UserId,
    pub language: String,
    pub judge_mode: RuntimeJudgeMode,
    pub sandbox_kind: RuntimeSandboxKind,
    pub source_code: String,
    pub limits: RuntimeLimits,
    pub testcases: Vec<RuntimeTestcase>,
    pub judge_config: Option<RuntimeJudgeConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OjJudgeValidatorConfig {
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub is_unordered: bool,
    pub is_token_mode: bool,
    pub is_float: bool,
    pub float_epsilon: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OjJudgeFunctionSignature {
    pub function_name: String,
    pub return_type: String,
    pub params: Vec<OjJudgeFunctionParameter>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OjJudgeFunctionParameter {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OjJudgeSpjConfig {
    pub language: String,
    pub source_code: String,
}

impl From<RuntimeValidatorConfig> for OjJudgeValidatorConfig {
    fn from(value: RuntimeValidatorConfig) -> Self {
        Self {
            ignore_whitespace: value.ignore_whitespace,
            ignore_case: value.ignore_case,
            is_unordered: value.is_unordered,
            is_token_mode: value.is_token_mode,
            is_float: value.is_float,
            float_epsilon: value.float_epsilon,
        }
    }
}

impl From<RuntimeFunctionSignature> for OjJudgeFunctionSignature {
    fn from(value: RuntimeFunctionSignature) -> Self {
        Self {
            function_name: value.function_name,
            return_type: value.return_type,
            params: value
                .params
                .into_iter()
                .map(|param| OjJudgeFunctionParameter {
                    name: param.name,
                    ty: param.ty,
                })
                .collect(),
        }
    }
}

impl From<RuntimeSpjConfig> for OjJudgeSpjConfig {
    fn from(value: RuntimeSpjConfig) -> Self {
        Self {
            language: value.language,
            source_code: value.source_code,
        }
    }
}
