use serde::{Deserialize, Serialize};

use nexus_shared::{ProblemId, SubmissionId, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskType {
    OjJudge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeJudgeMode {
    Acm,
    Functional,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSandboxKind {
    #[default]
    Nsjail,
    Wasm,
    NsjailWasm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTask {
    pub task_id: String,
    pub task_type: RuntimeTaskType,
    pub source_domain: String,
    pub source_entity_id: String,
    pub queue: String,
    pub lane: String,
    pub retry_policy: RuntimeRetryPolicy,
    pub payload: RuntimeTaskPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRetryPolicy {
    pub max_attempts: u32,
    pub retry_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeTaskPayload {
    OjJudge(OjJudgeTask),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OjJudgeTask {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLimits {
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTestcase {
    pub case_no: u32,
    pub input: String,
    pub expected_output: String,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeJudgeMethod {
    #[serde(alias = "standard")]
    Validator,
    Spj,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeJudgeConfig {
    pub judge_method: RuntimeJudgeMethod,
    pub validator: Option<RuntimeValidatorConfig>,
    pub spj: Option<RuntimeSpjConfig>,
    pub function_signature: Option<RuntimeFunctionSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeValidatorConfig {
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub is_unordered: bool,
    pub is_token_mode: bool,
    pub is_float: bool,
    pub float_epsilon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSpjConfig {
    pub language: String,
    pub source_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeFunctionSignature {
    pub function_name: String,
    pub return_type: String,
    pub params: Vec<RuntimeFunctionParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeFunctionParameter {
    pub name: String,
    pub ty: String,
}
