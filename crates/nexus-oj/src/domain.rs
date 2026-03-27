use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use nexus_shared::{ProblemId, SubmissionId, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeMode {
    Acm,
    Functional,
    EasyJudge,
}

impl JudgeMode {
    pub fn from_path(value: &str) -> Option<Self> {
        match value {
            "acm" => Some(Self::Acm),
            "functional" => Some(Self::Functional),
            "easy_judge" => Some(Self::EasyJudge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxKind {
    #[default]
    Nsjail,
    Wasm,
    NsjailWasm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub problem_id: ProblemId,
    pub title: String,
    pub slug: String,
    pub judge_mode: JudgeMode,
    #[serde(default)]
    pub sandbox_kind: SandboxKind,
    pub statement_md: String,
    pub supported_languages: Vec<String>,
    pub limits: BTreeMap<String, ProblemLimits>,
    pub testcases: Vec<Testcase>,
    pub judge_config: Option<JudgeConfig>,
    pub easy_config: Option<EasyProblemConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemSummary {
    pub problem_id: ProblemId,
    pub title: String,
    pub slug: String,
    pub judge_mode: JudgeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemDetail {
    pub problem: Problem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemLimits {
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Testcase {
    pub case_no: u32,
    pub input: String,
    pub expected_output: String,
    pub is_sample: bool,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeMethod {
    #[serde(alias = "standard")]
    Validator,
    Spj,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeConfig {
    pub judge_method: JudgeMethod,
    pub validator: Option<ValidatorConfig>,
    pub spj: Option<SpjConfig>,
    pub function_signature: Option<FunctionSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorConfig {
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub is_unordered: bool,
    pub is_token_mode: bool,
    pub is_float: bool,
    pub float_epsilon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpjConfig {
    pub language: String,
    pub source_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSignature {
    pub function_name: String,
    pub return_type: String,
    pub params: Vec<FunctionParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionParameter {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionDraft {
    pub problem_id: ProblemId,
    pub user_id: UserId,
    pub language: String,
    pub source_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EasySubmissionDraft {
    pub problem_id: ProblemId,
    pub user_id: UserId,
    pub answer: EasyAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EasyQuestionType {
    TrueFalse,
    SingleChoice,
    MultipleChoice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EasyOption {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EasyProblemConfig {
    pub question_type: EasyQuestionType,
    pub options: Vec<EasyOption>,
    pub standard_answer: EasyAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EasyAnswer {
    Text(String),
    Options(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Pending,
    Queued,
    Running,
    Accepted,
    WrongAnswer,
    CompileError,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    SecurityViolation,
    RuntimeError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionRecord {
    pub submission_id: SubmissionId,
    pub problem_id: ProblemId,
    pub user_id: UserId,
    pub language: String,
    pub status: SubmissionStatus,
    pub score: u32,
    pub max_score: u32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionCaseStatus {
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    SecurityViolation,
    RuntimeError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionCaseResult {
    pub case_no: u32,
    pub status: SubmissionCaseStatus,
    pub score: u32,
    pub time_used_ms: u64,
    pub memory_used_kb: u64,
    pub actual_output: String,
    pub expected_output_snapshot: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionResult {
    pub submission_id: SubmissionId,
    pub overall_status: SubmissionStatus,
    pub compile_output: Option<String>,
    pub runtime_output: Option<String>,
    pub compile_time_ms: u64,
    pub judge_compile_time_ms: u64,
    pub run_time_ms: u64,
    pub time_used_ms: u64,
    pub memory_used_kb: u64,
    pub judge_summary: Option<String>,
    pub case_results: Vec<SubmissionCaseResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionDetail {
    pub submission: SubmissionRecord,
    pub source_code: String,
    pub result: Option<SubmissionResult>,
}
