use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use nexus_jobs::{
    build_oj_judge_job, map_job_to_runtime_task, JobDefinition, JobRetryPolicy, JobRoute,
    OjJudgeJobInput,
};
use nexus_runtime::{
    RuntimeCaseFinalStatus, RuntimeExecutionOutcome, RuntimeFailureKind, RuntimeFunctionParameter,
    RuntimeFunctionSignature, RuntimeJudgeConfig, RuntimeJudgeMethod, RuntimeJudgeMode,
    RuntimeLimits, RuntimeSandboxKind, RuntimeSpjConfig, RuntimeStageStatus, RuntimeTask,
    RuntimeTaskEvent, RuntimeTaskLifecycleStatus, RuntimeTestcase, RuntimeValidatorConfig,
};

use crate::{
    domain::{
        EasyAnswer, EasyProblemConfig, EasyQuestionType, EasySubmissionDraft, JudgeMethod,
        JudgeMode, Problem, ProblemDetail, ProblemLimits, ProblemSummary, SandboxKind, SpjConfig,
        SubmissionCaseResult, SubmissionCaseStatus, SubmissionDetail, SubmissionDraft,
        SubmissionRecord, SubmissionResult, SubmissionStatus, ValidatorConfig,
    },
    repository::{ProblemRepository, SubmissionRepository},
};
use nexus_shared::{AppError, AppResult, ProblemId, SubmissionId};

pub struct OjService {
    problem_repository: Arc<dyn ProblemRepository>,
    submission_repository: Arc<dyn SubmissionRepository>,
}

impl OjService {
    pub fn new(
        problem_repository: Arc<dyn ProblemRepository>,
        submission_repository: Arc<dyn SubmissionRepository>,
    ) -> Self {
        Self {
            problem_repository,
            submission_repository,
        }
    }

    pub async fn list_problem_summaries(&self) -> AppResult<Vec<ProblemSummary>> {
        self.problem_repository.list_summaries().await
    }

    pub async fn get_problem_detail(&self, problem_id: &str) -> AppResult<ProblemDetail> {
        self.problem_repository
            .find_detail(problem_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("problem not found: {problem_id}")))
    }

    pub async fn save_problem(&self, problem: Problem) -> AppResult<()> {
        validate_problem(&problem)?;
        self.problem_repository.save(problem).await
    }

    pub async fn validate_submission(&self, draft: &SubmissionDraft) -> AppResult<()> {
        self.submission_repository.validate_submission(draft).await
    }

    pub async fn create_submission(&self, draft: SubmissionDraft) -> AppResult<SubmissionRecord> {
        let problem = self.get_problem_detail(&draft.problem_id.0).await?.problem;
        validate_problem(&problem)?;

        if matches!(problem.judge_mode, JudgeMode::EasyJudge) {
            return Err(AppError::BadRequest(
                "easy_judge problems must use /api/v1/oj/easy-judge/submissions".to_owned(),
            ));
        }

        if !problem
            .supported_languages
            .iter()
            .any(|language| language == &draft.language)
        {
            return Err(AppError::BadRequest(format!(
                "language {} is not supported for problem {}",
                draft.language, problem.problem_id.0
            )));
        }

        if !problem.limits.contains_key(&draft.language) {
            return Err(AppError::BadRequest(format!(
                "language {} is not configured for problem {}",
                draft.language, problem.problem_id.0
            )));
        }

        self.submission_repository
            .validate_submission(&draft)
            .await?;
        self.submission_repository.create_submission(draft).await
    }

    pub async fn list_submissions(&self) -> AppResult<Vec<SubmissionRecord>> {
        self.submission_repository.list_submissions().await
    }

    pub async fn get_submission_detail(&self, submission_id: &str) -> AppResult<SubmissionDetail> {
        self.submission_repository
            .find_submission(submission_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("submission not found: {submission_id}")))
    }

    pub async fn build_runtime_task(&self, submission_id: &str) -> AppResult<RuntimeTask> {
        let job = self.build_job_definition(submission_id).await?;
        map_job_to_runtime_task(&job)
    }

    pub async fn build_job_definition(&self, submission_id: &str) -> AppResult<JobDefinition> {
        let submission = self.get_submission_detail(submission_id).await?;
        let problem = self
            .get_problem_detail(&submission.submission.problem_id.0)
            .await?;
        let problem = problem.problem;

        validate_problem(&problem)?;

        if matches!(problem.judge_mode, JudgeMode::EasyJudge) {
            return Err(AppError::BadRequest(
                "easy_judge submissions do not produce runtime tasks".to_owned(),
            ));
        }

        let limits = problem
            .limits
            .get(&submission.submission.language)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "language {} is not configured for problem {}",
                    submission.submission.language, problem.problem_id.0
                ))
            })?;
        let lane = derive_runtime_lane(&problem, &submission.submission.language);
        let judge_config = problem.judge_config.clone().map(map_runtime_judge_config);

        Ok(build_oj_judge_job(OjJudgeJobInput {
            job_id: format!("task-{}", submission.submission.submission_id.0),
            source_entity_id: submission.submission.submission_id.0.clone(),
            submission_id: submission.submission.submission_id,
            problem_id: problem.problem_id,
            user_id: submission.submission.user_id,
            language: submission.submission.language,
            judge_mode: map_runtime_judge_mode(&problem.judge_mode)?,
            sandbox_kind: map_runtime_sandbox_kind(problem.sandbox_kind),
            source_code: submission.source_code,
            limits: RuntimeLimits {
                time_limit_ms: limits.time_limit_ms,
                memory_limit_kb: limits.memory_limit_kb,
            },
            testcases: problem
                .testcases
                .into_iter()
                .map(|testcase| RuntimeTestcase {
                    case_no: testcase.case_no,
                    input: testcase.input,
                    expected_output: testcase.expected_output,
                    score: testcase.score,
                })
                .collect(),
            judge_config,
            route: JobRoute {
                queue: "oj_judge".to_owned(),
                lane,
            },
            retry_policy: JobRetryPolicy {
                max_attempts: 3,
                retry_delay_ms: 1_000,
            },
        }))
    }

    pub async fn judge_easy_submission(
        &self,
        draft: EasySubmissionDraft,
    ) -> AppResult<SubmissionRecord> {
        let detail = self.get_problem_detail(&draft.problem_id.0).await?;
        let problem = detail.problem;

        if !matches!(problem.judge_mode, JudgeMode::EasyJudge) {
            return Err(AppError::BadRequest(
                "problem is not an easy_judge problem".to_owned(),
            ));
        }

        let easy_config = problem
            .easy_config
            .ok_or_else(|| AppError::BadRequest("easy_config is missing".to_owned()))?;
        let result = judge_easy_answer(&easy_config, &draft.answer)?;
        let stored_answer = serde_json::to_string(&draft.answer)
            .map_err(|err| AppError::BadRequest(err.to_string()))?;

        self.submission_repository
            .create_direct_submission(
                draft,
                result.status,
                result.score,
                result.max_score,
                result.message,
                stored_answer,
            )
            .await
    }

    pub async fn apply_runtime_event(&self, event: &RuntimeTaskEvent) -> AppResult<()> {
        self.submission_repository.apply_runtime_event(event).await
    }
}

pub struct InMemoryProblemRepository {
    problems: RwLock<Vec<Problem>>,
}

impl InMemoryProblemRepository {
    pub fn seeded() -> Self {
        let mut limits = BTreeMap::new();
        limits.insert(
            "cpp".to_owned(),
            ProblemLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 50 * 1024,
            },
        );
        limits.insert(
            "python".to_owned(),
            ProblemLimits {
                time_limit_ms: 2000,
                memory_limit_kb: 100 * 1024,
            },
        );
        limits.insert(
            "rust".to_owned(),
            ProblemLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 50 * 1024,
            },
        );

        Self {
            problems: RwLock::new(vec![Problem {
                problem_id: ProblemId::from("two-sum"),
                title: "两数之和".to_owned(),
                slug: "two-sum".to_owned(),
                judge_mode: JudgeMode::Acm,
                sandbox_kind: SandboxKind::Nsjail,
                statement_md: "给定一个整数数组，输出两数之和。".to_owned(),
                supported_languages: vec!["cpp".to_owned(), "python".to_owned(), "rust".to_owned()],
                limits,
                testcases: vec![crate::domain::Testcase {
                    case_no: 1,
                    input: "1 2\n".to_owned(),
                    expected_output: "3\n".to_owned(),
                    is_sample: true,
                    score: 100,
                }],
                judge_config: None,
                easy_config: None,
            }]),
        }
    }
}

#[async_trait]
impl ProblemRepository for InMemoryProblemRepository {
    async fn list_summaries(&self) -> AppResult<Vec<ProblemSummary>> {
        let guard = self.problems.read().map_err(|_| AppError::Internal)?;

        Ok(guard
            .iter()
            .map(|problem| ProblemSummary {
                problem_id: problem.problem_id.clone(),
                title: problem.title.clone(),
                slug: problem.slug.clone(),
                judge_mode: problem.judge_mode.clone(),
            })
            .collect())
    }

    async fn find_detail(&self, problem_id: &str) -> AppResult<Option<ProblemDetail>> {
        let guard = self.problems.read().map_err(|_| AppError::Internal)?;

        Ok(guard
            .iter()
            .find(|problem| problem.problem_id.0 == problem_id)
            .cloned()
            .map(|problem| ProblemDetail { problem }))
    }

    async fn save(&self, problem: Problem) -> AppResult<()> {
        let mut guard = self.problems.write().map_err(|_| AppError::Internal)?;

        if let Some(existing) = guard
            .iter_mut()
            .find(|existing| existing.problem_id == problem.problem_id)
        {
            *existing = problem;
        } else {
            guard.push(problem);
        }

        Ok(())
    }
}

pub struct InMemorySubmissionRepository {
    submissions: RwLock<Vec<SubmissionDetail>>,
}

impl Default for InMemorySubmissionRepository {
    fn default() -> Self {
        Self {
            submissions: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SubmissionRepository for InMemorySubmissionRepository {
    async fn validate_submission(&self, draft: &SubmissionDraft) -> AppResult<()> {
        if draft.source_code.trim().is_empty() {
            return Err(AppError::BadRequest(
                "submission source code cannot be empty".to_owned(),
            ));
        }

        Ok(())
    }

    async fn create_submission(&self, draft: SubmissionDraft) -> AppResult<SubmissionRecord> {
        let submission = SubmissionRecord {
            submission_id: SubmissionId::from(format!(
                "sub-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| AppError::Internal)?
                    .as_millis()
            )),
            problem_id: draft.problem_id,
            user_id: draft.user_id,
            language: draft.language,
            status: SubmissionStatus::Pending,
            score: 0,
            max_score: 0,
            message: None,
        };

        let mut guard = self.submissions.write().map_err(|_| AppError::Internal)?;
        guard.push(SubmissionDetail {
            submission: submission.clone(),
            source_code: draft.source_code,
            result: None,
        });
        Ok(submission)
    }

    async fn apply_runtime_event(&self, event: &RuntimeTaskEvent) -> AppResult<()> {
        let Some(submission_id) = &event.submission_id else {
            return Ok(());
        };

        let (status, score, max_score, message) = derive_submission_projection(event);
        let mut guard = self.submissions.write().map_err(|_| AppError::Internal)?;
        let Some(detail) = guard
            .iter_mut()
            .find(|detail| detail.submission.submission_id.0 == *submission_id)
        else {
            return Ok(());
        };

        detail.submission.status = status.clone();
        detail.submission.score = score;
        detail.submission.max_score = max_score;
        detail.submission.message = message;
        detail.result = Some(build_runtime_submission_result(&detail.submission, event));
        Ok(())
    }

    async fn list_submissions(&self) -> AppResult<Vec<SubmissionRecord>> {
        let guard = self.submissions.read().map_err(|_| AppError::Internal)?;
        Ok(guard
            .iter()
            .map(|detail| detail.submission.clone())
            .collect())
    }

    async fn find_submission(&self, submission_id: &str) -> AppResult<Option<SubmissionDetail>> {
        let guard = self.submissions.read().map_err(|_| AppError::Internal)?;
        Ok(guard
            .iter()
            .find(|detail| detail.submission.submission_id.0 == submission_id)
            .cloned())
    }

    async fn create_direct_submission(
        &self,
        draft: EasySubmissionDraft,
        status: SubmissionStatus,
        score: u32,
        max_score: u32,
        message: Option<String>,
        _stored_answer: String,
    ) -> AppResult<SubmissionRecord> {
        let submission = SubmissionRecord {
            submission_id: SubmissionId::from(format!(
                "sub-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| AppError::Internal)?
                    .as_millis()
            )),
            problem_id: draft.problem_id,
            user_id: draft.user_id,
            language: "easy_judge".to_owned(),
            status,
            score,
            max_score,
            message,
        };

        let mut guard = self.submissions.write().map_err(|_| AppError::Internal)?;
        guard.push(SubmissionDetail {
            submission: submission.clone(),
            source_code: _stored_answer,
            result: Some(SubmissionResult {
                submission_id: submission.submission_id.clone(),
                overall_status: submission.status.clone(),
                compile_output: None,
                runtime_output: submission.message.clone(),
                compile_time_ms: 0,
                judge_compile_time_ms: 0,
                run_time_ms: 0,
                time_used_ms: 0,
                memory_used_kb: 0,
                judge_summary: submission.message.clone(),
                case_results: Vec::new(),
            }),
        });
        Ok(submission)
    }
}

fn validate_problem(problem: &Problem) -> AppResult<()> {
    if problem.problem_id.0.trim().is_empty() {
        return Err(AppError::BadRequest(
            "problem_id cannot be empty".to_owned(),
        ));
    }

    if problem.title.trim().is_empty() {
        return Err(AppError::BadRequest("title cannot be empty".to_owned()));
    }

    if problem.slug.trim().is_empty() {
        return Err(AppError::BadRequest("slug cannot be empty".to_owned()));
    }

    if !matches!(problem.judge_mode, JudgeMode::EasyJudge) && problem.supported_languages.is_empty()
    {
        return Err(AppError::BadRequest(
            "supported_languages cannot be empty".to_owned(),
        ));
    }

    if problem.limits.is_empty() && !matches!(problem.judge_mode, JudgeMode::EasyJudge) {
        return Err(AppError::BadRequest("limits cannot be empty".to_owned()));
    }

    if matches!(problem.judge_mode, JudgeMode::EasyJudge) && problem.easy_config.is_none() {
        return Err(AppError::BadRequest(
            "easy_judge problem requires easy_config".to_owned(),
        ));
    }

    if matches!(
        problem.sandbox_kind,
        SandboxKind::Wasm | SandboxKind::NsjailWasm
    ) {
        if matches!(problem.judge_mode, JudgeMode::EasyJudge) {
            return Err(AppError::BadRequest(
                "easy_judge problems do not support wasm sandbox".to_owned(),
            ));
        }

        if let Some(language) = problem
            .supported_languages
            .iter()
            .find(|language| !matches!(language.as_str(), "cpp" | "rust"))
        {
            return Err(AppError::BadRequest(format!(
                "wasm sandbox currently supports only cpp/rust, found: {language}"
            )));
        }

        if problem
            .judge_config
            .as_ref()
            .is_some_and(|config| matches!(config.judge_method, JudgeMethod::Spj))
        {
            return Err(AppError::BadRequest(
                "wasm sandbox currently supports validator-based judging only".to_owned(),
            ));
        }
    }

    if !matches!(problem.judge_mode, JudgeMode::EasyJudge) && problem.testcases.is_empty() {
        return Err(AppError::BadRequest(
            "non-easy problems require at least one testcase".to_owned(),
        ));
    }

    validate_judge_config(problem)?;

    Ok(())
}

fn validate_judge_config(problem: &Problem) -> AppResult<()> {
    let Some(config) = problem.judge_config.as_ref() else {
        return Ok(());
    };

    match config.judge_method {
        JudgeMethod::Validator => {
            validate_validator_config(config.validator.as_ref(), config.spj.as_ref())?
        }
        JudgeMethod::Spj => validate_spj_config(config.spj.as_ref())?,
    }

    Ok(())
}

fn validate_validator_config(
    config: Option<&ValidatorConfig>,
    spj: Option<&SpjConfig>,
) -> AppResult<()> {
    if spj.is_some() {
        return Err(AppError::BadRequest(
            "validator judge_method must not define spj config".to_owned(),
        ));
    }

    let Some(config) = config else {
        return Ok(());
    };

    if config.float_epsilon < 0.0 {
        return Err(AppError::BadRequest(
            "validator float_epsilon cannot be negative".to_owned(),
        ));
    }

    Ok(())
}

fn validate_spj_config(config: Option<&SpjConfig>) -> AppResult<()> {
    let Some(config) = config else {
        return Err(AppError::BadRequest(
            "spj judge_method requires spj config".to_owned(),
        ));
    };

    if config.language.trim().is_empty() {
        return Err(AppError::BadRequest(
            "spj language cannot be empty".to_owned(),
        ));
    }

    if config.source_code.trim().is_empty() {
        return Err(AppError::BadRequest(
            "spj source_code cannot be empty".to_owned(),
        ));
    }

    Ok(())
}

fn map_runtime_judge_mode(mode: &JudgeMode) -> AppResult<RuntimeJudgeMode> {
    match mode {
        JudgeMode::Acm => Ok(RuntimeJudgeMode::Acm),
        JudgeMode::Functional => Ok(RuntimeJudgeMode::Functional),
        JudgeMode::EasyJudge => Err(AppError::BadRequest(
            "easy_judge does not map to runtime judge mode".to_owned(),
        )),
    }
}

fn map_runtime_sandbox_kind(kind: SandboxKind) -> RuntimeSandboxKind {
    match kind {
        SandboxKind::Nsjail => RuntimeSandboxKind::Nsjail,
        SandboxKind::Wasm => RuntimeSandboxKind::Wasm,
        SandboxKind::NsjailWasm => RuntimeSandboxKind::NsjailWasm,
    }
}

fn derive_runtime_lane(problem: &Problem, language: &str) -> String {
    if matches!(problem.judge_mode, JudgeMode::Functional) {
        return "heavy".to_owned();
    }

    if problem
        .judge_config
        .as_ref()
        .is_some_and(|config| matches!(config.judge_method, JudgeMethod::Spj))
    {
        return "special".to_owned();
    }

    if language == "python" {
        return "normal".to_owned();
    }

    "fast".to_owned()
}

pub(crate) fn derive_submission_projection(
    event: &RuntimeTaskEvent,
) -> (SubmissionStatus, u32, u32, Option<String>) {
    match event.status {
        RuntimeTaskLifecycleStatus::Queued
        | RuntimeTaskLifecycleStatus::Retrying
        | RuntimeTaskLifecycleStatus::Preparing
        | RuntimeTaskLifecycleStatus::Prepared
        | RuntimeTaskLifecycleStatus::Compiling => {
            (SubmissionStatus::Queued, 0, 0, Some(event.message.clone()))
        }
        RuntimeTaskLifecycleStatus::Running => {
            (SubmissionStatus::Running, 0, 0, Some(event.message.clone()))
        }
        RuntimeTaskLifecycleStatus::Completed => {
            let max_score = total_score(event.outcome.as_ref());
            let score = accepted_score(event.outcome.as_ref());
            (
                SubmissionStatus::Accepted,
                score,
                max_score,
                Some("accepted".to_owned()),
            )
        }
        RuntimeTaskLifecycleStatus::Failed => {
            let (status, message) = classify_failure(event.outcome.as_ref(), &event.message);
            let max_score = total_score(event.outcome.as_ref());
            let score = accepted_score(event.outcome.as_ref());
            (status, score, max_score, Some(message))
        }
        RuntimeTaskLifecycleStatus::DeadLettered => (
            SubmissionStatus::RuntimeError,
            accepted_score(event.outcome.as_ref()),
            total_score(event.outcome.as_ref()),
            Some(event.message.clone()),
        ),
    }
}

fn total_score(outcome: Option<&RuntimeExecutionOutcome>) -> u32 {
    outcome
        .map(|item| item.cases.iter().map(|case| case.score).sum())
        .unwrap_or(0)
}

fn accepted_score(outcome: Option<&RuntimeExecutionOutcome>) -> u32 {
    outcome
        .map(|item| {
            item.cases
                .iter()
                .filter(|case| matches!(case.status, RuntimeCaseFinalStatus::Accepted))
                .map(|case| case.score)
                .sum()
        })
        .unwrap_or(0)
}

fn classify_failure(
    outcome: Option<&RuntimeExecutionOutcome>,
    fallback_message: &str,
) -> (SubmissionStatus, String) {
    if let Some(outcome) = outcome {
        if let Some(judge_compile) = &outcome.judge_compile {
            if matches!(judge_compile.status, RuntimeStageStatus::Failed) {
                return (
                    map_compile_failure_status(judge_compile.failure_kind),
                    if judge_compile.stderr_excerpt.is_empty() {
                        fallback_message.to_owned()
                    } else {
                        judge_compile.stderr_excerpt.clone()
                    },
                );
            }
        }

        if let Some(compile) = &outcome.compile {
            if matches!(compile.status, RuntimeStageStatus::Failed) {
                return (
                    map_compile_failure_status(compile.failure_kind),
                    if compile.stderr_excerpt.is_empty() {
                        fallback_message.to_owned()
                    } else {
                        compile.stderr_excerpt.clone()
                    },
                );
            }
        }

        if outcome
            .cases
            .iter()
            .any(|case| matches!(case.status, RuntimeCaseFinalStatus::WrongAnswer))
        {
            return (SubmissionStatus::WrongAnswer, "wrong answer".to_owned());
        }

        if let Some(failed_case) = outcome
            .cases
            .iter()
            .find(|case| !matches!(case.status, RuntimeCaseFinalStatus::Accepted))
        {
            return (
                map_submission_case_failure_status(&failed_case.status),
                if failed_case.stderr_excerpt.is_empty() {
                    fallback_message.to_owned()
                } else {
                    failed_case.stderr_excerpt.clone()
                },
            );
        }
    }

    (SubmissionStatus::RuntimeError, fallback_message.to_owned())
}

fn build_runtime_submission_result(
    submission: &SubmissionRecord,
    event: &RuntimeTaskEvent,
) -> SubmissionResult {
    let case_results = event
        .outcome
        .as_ref()
        .map(|outcome| {
            outcome
                .cases
                .iter()
                .map(|case| SubmissionCaseResult {
                    case_no: case.case_no,
                    status: map_case_status(&case.status),
                    score: case.score,
                    time_used_ms: case.duration_ms as u64,
                    memory_used_kb: case.memory_used_kb,
                    actual_output: case.stdout_excerpt.clone(),
                    expected_output_snapshot: String::new(),
                    message: if case.stderr_excerpt.is_empty() {
                        None
                    } else {
                        Some(case.stderr_excerpt.clone())
                    },
                })
                .collect()
        })
        .unwrap_or_default();

    SubmissionResult {
        submission_id: submission.submission_id.clone(),
        overall_status: submission.status.clone(),
        compile_output: event.outcome.as_ref().and_then(|outcome| {
            outcome
                .judge_compile
                .as_ref()
                .or(outcome.compile.as_ref())
                .and_then(|stage| {
                    if stage.stderr_excerpt.is_empty() {
                        None
                    } else {
                        Some(stage.stderr_excerpt.clone())
                    }
                })
        }),
        runtime_output: event.outcome.as_ref().and_then(|outcome| {
            outcome.cases.iter().find_map(|case| {
                if !case.stderr_excerpt.is_empty() {
                    Some(case.stderr_excerpt.clone())
                } else if !case.stdout_excerpt.is_empty() {
                    Some(case.stdout_excerpt.clone())
                } else {
                    None
                }
            })
        }),
        compile_time_ms: event.outcome.as_ref().map(total_compile_time).unwrap_or(0),
        judge_compile_time_ms: event
            .outcome
            .as_ref()
            .map(total_judge_compile_time)
            .unwrap_or(0),
        run_time_ms: event.outcome.as_ref().map(total_case_time).unwrap_or(0),
        time_used_ms: event.outcome.as_ref().map(total_case_time).unwrap_or(0),
        memory_used_kb: event.outcome.as_ref().map(total_case_memory).unwrap_or(0),
        judge_summary: submission
            .message
            .clone()
            .or_else(|| Some(event.message.clone())),
        case_results,
    }
}

fn map_case_status(status: &RuntimeCaseFinalStatus) -> SubmissionCaseStatus {
    match status {
        RuntimeCaseFinalStatus::Accepted => SubmissionCaseStatus::Accepted,
        RuntimeCaseFinalStatus::WrongAnswer => SubmissionCaseStatus::WrongAnswer,
        RuntimeCaseFinalStatus::TimeLimitExceeded => SubmissionCaseStatus::TimeLimitExceeded,
        RuntimeCaseFinalStatus::MemoryLimitExceeded => SubmissionCaseStatus::MemoryLimitExceeded,
        RuntimeCaseFinalStatus::OutputLimitExceeded => SubmissionCaseStatus::OutputLimitExceeded,
        RuntimeCaseFinalStatus::SecurityViolation => SubmissionCaseStatus::SecurityViolation,
        RuntimeCaseFinalStatus::RuntimeError => SubmissionCaseStatus::RuntimeError,
    }
}

fn map_compile_failure_status(failure_kind: Option<RuntimeFailureKind>) -> SubmissionStatus {
    match failure_kind {
        Some(RuntimeFailureKind::TimeLimitExceeded) => SubmissionStatus::TimeLimitExceeded,
        Some(RuntimeFailureKind::MemoryLimitExceeded) => SubmissionStatus::MemoryLimitExceeded,
        Some(RuntimeFailureKind::OutputLimitExceeded) => SubmissionStatus::OutputLimitExceeded,
        Some(RuntimeFailureKind::SecurityViolation) => SubmissionStatus::SecurityViolation,
        _ => SubmissionStatus::CompileError,
    }
}

fn map_submission_case_failure_status(status: &RuntimeCaseFinalStatus) -> SubmissionStatus {
    match status {
        RuntimeCaseFinalStatus::Accepted => SubmissionStatus::Accepted,
        RuntimeCaseFinalStatus::WrongAnswer => SubmissionStatus::WrongAnswer,
        RuntimeCaseFinalStatus::TimeLimitExceeded => SubmissionStatus::TimeLimitExceeded,
        RuntimeCaseFinalStatus::MemoryLimitExceeded => SubmissionStatus::MemoryLimitExceeded,
        RuntimeCaseFinalStatus::OutputLimitExceeded => SubmissionStatus::OutputLimitExceeded,
        RuntimeCaseFinalStatus::SecurityViolation => SubmissionStatus::SecurityViolation,
        RuntimeCaseFinalStatus::RuntimeError => SubmissionStatus::RuntimeError,
    }
}

fn total_case_time(outcome: &RuntimeExecutionOutcome) -> u64 {
    outcome
        .cases
        .iter()
        .map(|case| case.duration_ms as u64)
        .sum()
}

fn total_compile_time(outcome: &RuntimeExecutionOutcome) -> u64 {
    outcome
        .compile
        .as_ref()
        .map(|stage| stage.duration_ms as u64)
        .unwrap_or(0)
}

fn total_judge_compile_time(outcome: &RuntimeExecutionOutcome) -> u64 {
    outcome
        .judge_compile
        .as_ref()
        .map(|stage| stage.duration_ms as u64)
        .unwrap_or(0)
}

fn total_case_memory(outcome: &RuntimeExecutionOutcome) -> u64 {
    outcome
        .cases
        .iter()
        .map(|case| case.memory_used_kb)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{derive_submission_projection, validate_problem};
    use crate::domain::{
        JudgeConfig, JudgeMethod, JudgeMode, Problem, ProblemLimits, SandboxKind, SpjConfig,
        SubmissionStatus, Testcase, ValidatorConfig,
    };
    use crate::{InMemoryProblemRepository, InMemorySubmissionRepository, OjService};
    use nexus_runtime::{
        RuntimeCaseFinalStatus, RuntimeCaseOutcome, RuntimeExecutionOutcome, RuntimeFailureKind,
        RuntimeSandboxKind, RuntimeStageOutcome, RuntimeStageStatus, RuntimeTaskEvent,
        RuntimeTaskLifecycleStatus, RuntimeTaskPayload,
    };
    use nexus_shared::{ProblemId, UserId};
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};

    #[test]
    fn accepts_default_validator_without_explicit_config() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Validator,
            validator: None,
            spj: None,
            function_signature: None,
        }));

        validate_problem(&problem).expect("default validator config should be allowed");
    }

    #[test]
    fn rejects_validator_judge_with_spj_config() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Validator,
            validator: None,
            spj: Some(SpjConfig {
                language: "cpp".to_owned(),
                source_code: "return 0;".to_owned(),
            }),
            function_signature: None,
        }));

        let error = validate_problem(&problem).expect_err("validator must not accept spj config");
        assert!(format!("{error}").contains("validator judge_method must not define spj config"));
    }

    #[test]
    fn rejects_negative_validator_epsilon() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Validator,
            validator: Some(ValidatorConfig {
                ignore_whitespace: true,
                ignore_case: false,
                is_unordered: false,
                is_token_mode: false,
                is_float: true,
                float_epsilon: -1.0,
            }),
            spj: None,
            function_signature: None,
        }));

        let error = validate_problem(&problem).expect_err("negative epsilon should be rejected");
        assert!(format!("{error}").contains("validator float_epsilon cannot be negative"));
    }

    #[test]
    fn rejects_spj_without_source_code() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Spj,
            validator: None,
            spj: Some(SpjConfig {
                language: "cpp".to_owned(),
                source_code: "   ".to_owned(),
            }),
            function_signature: None,
        }));

        let error = validate_problem(&problem).expect_err("empty spj source should be rejected");
        assert!(format!("{error}").contains("spj source_code cannot be empty"));
    }

    #[test]
    fn accepts_valid_spj_config() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Spj,
            validator: None,
            spj: Some(SpjConfig {
                language: "python".to_owned(),
                source_code: "print(0)".to_owned(),
            }),
            function_signature: None,
        }));

        validate_problem(&problem).expect("valid spj config should pass");
    }

    #[test]
    fn runtime_projection_maps_accepted_outcome() {
        let (status, score, max_score, message) = derive_submission_projection(&runtime_event(
            "sub-1",
            RuntimeTaskLifecycleStatus::Completed,
            Some(RuntimeExecutionOutcome {
                compile: None,
                judge_compile: None,
                cases: vec![
                    runtime_case(1, 30, RuntimeCaseFinalStatus::Accepted),
                    runtime_case(2, 70, RuntimeCaseFinalStatus::Accepted),
                ],
                final_status: RuntimeTaskLifecycleStatus::Completed,
            }),
            "runtime execution completed successfully",
        ));

        assert_eq!(status, SubmissionStatus::Accepted);
        assert_eq!(score, 100);
        assert_eq!(max_score, 100);
        assert_eq!(message.as_deref(), Some("accepted"));
    }

    #[test]
    fn runtime_projection_maps_compile_error() {
        let (status, score, max_score, message) = derive_submission_projection(&runtime_event(
            "sub-1",
            RuntimeTaskLifecycleStatus::Failed,
            Some(RuntimeExecutionOutcome {
                compile: Some(RuntimeStageOutcome {
                    stage_name: "compile".to_owned(),
                    status: RuntimeStageStatus::Failed,
                    exit_code: Some(1),
                    signal: None,
                    failure_kind: None,
                    duration_ms: 5,
                    memory_used_kb: 12_288,
                    stdout_size_bytes: 0,
                    stderr_size_bytes: 14,
                    stdout_path: String::new(),
                    stderr_path: String::new(),
                    stdout_excerpt: String::new(),
                    stderr_excerpt: "compile failed".to_owned(),
                }),
                judge_compile: None,
                cases: Vec::new(),
                final_status: RuntimeTaskLifecycleStatus::Failed,
            }),
            "runtime execution failed",
        ));

        assert_eq!(status, SubmissionStatus::CompileError);
        assert_eq!(score, 0);
        assert_eq!(max_score, 0);
        assert_eq!(message.as_deref(), Some("compile failed"));
    }

    #[test]
    fn runtime_projection_maps_wrong_answer() {
        let (status, score, max_score, message) = derive_submission_projection(&runtime_event(
            "sub-1",
            RuntimeTaskLifecycleStatus::Failed,
            Some(RuntimeExecutionOutcome {
                compile: None,
                judge_compile: None,
                cases: vec![
                    runtime_case(1, 40, RuntimeCaseFinalStatus::Accepted),
                    runtime_case(2, 60, RuntimeCaseFinalStatus::WrongAnswer),
                ],
                final_status: RuntimeTaskLifecycleStatus::Failed,
            }),
            "runtime execution failed",
        ));

        assert_eq!(status, SubmissionStatus::WrongAnswer);
        assert_eq!(score, 40);
        assert_eq!(max_score, 100);
        assert_eq!(message.as_deref(), Some("wrong answer"));
    }

    #[test]
    fn runtime_projection_maps_runtime_error() {
        let (status, score, max_score, message) = derive_submission_projection(&runtime_event(
            "sub-1",
            RuntimeTaskLifecycleStatus::Failed,
            Some(RuntimeExecutionOutcome {
                compile: None,
                judge_compile: None,
                cases: vec![RuntimeCaseOutcome {
                    case_no: 1,
                    score: 100,
                    status: RuntimeCaseFinalStatus::RuntimeError,
                    exit_code: Some(1),
                    duration_ms: 12,
                    memory_used_kb: 10_240,
                    stdout_path: String::new(),
                    stderr_path: String::new(),
                    stdout_excerpt: String::new(),
                    stderr_excerpt: "segmentation fault".to_owned(),
                }],
                final_status: RuntimeTaskLifecycleStatus::Failed,
            }),
            "runtime execution failed",
        ));

        assert_eq!(status, SubmissionStatus::RuntimeError);
        assert_eq!(score, 0);
        assert_eq!(max_score, 100);
        assert_eq!(message.as_deref(), Some("segmentation fault"));
    }

    #[test]
    fn runtime_projection_maps_time_limit_exceeded() {
        let (status, score, max_score, message) = derive_submission_projection(&runtime_event(
            "sub-1",
            RuntimeTaskLifecycleStatus::Failed,
            Some(RuntimeExecutionOutcome {
                compile: None,
                judge_compile: None,
                cases: vec![runtime_case(
                    1,
                    100,
                    RuntimeCaseFinalStatus::TimeLimitExceeded,
                )],
                final_status: RuntimeTaskLifecycleStatus::Failed,
            }),
            "runtime execution failed",
        ));

        assert_eq!(status, SubmissionStatus::TimeLimitExceeded);
        assert_eq!(score, 0);
        assert_eq!(max_score, 100);
        assert_eq!(message.as_deref(), Some("runtime execution failed"));
    }

    #[test]
    fn runtime_projection_maps_compile_security_violation() {
        let (status, score, max_score, message) = derive_submission_projection(&runtime_event(
            "sub-1",
            RuntimeTaskLifecycleStatus::Failed,
            Some(RuntimeExecutionOutcome {
                compile: Some(RuntimeStageOutcome {
                    stage_name: "compile".to_owned(),
                    status: RuntimeStageStatus::Failed,
                    exit_code: Some(159),
                    signal: Some(31),
                    failure_kind: Some(RuntimeFailureKind::SecurityViolation),
                    duration_ms: 2,
                    memory_used_kb: 4_096,
                    stdout_size_bytes: 0,
                    stderr_size_bytes: 22,
                    stdout_path: String::new(),
                    stderr_path: String::new(),
                    stdout_excerpt: String::new(),
                    stderr_excerpt: "operation not permitted".to_owned(),
                }),
                judge_compile: None,
                cases: Vec::new(),
                final_status: RuntimeTaskLifecycleStatus::Failed,
            }),
            "runtime execution failed",
        ));

        assert_eq!(status, SubmissionStatus::SecurityViolation);
        assert_eq!(score, 0);
        assert_eq!(max_score, 0);
        assert_eq!(message.as_deref(), Some("operation not permitted"));
    }

    #[tokio::test]
    async fn submission_detail_exposes_runtime_result_after_event_application() {
        let service = OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        );

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("two-sum"),
                user_id: UserId::from("u-1"),
                language: "cpp".to_owned(),
                source_code: "int main() { return 0; }".to_owned(),
            })
            .await
            .expect("submission should be created");

        service
            .apply_runtime_event(&runtime_event(
                &submission.submission_id.0,
                RuntimeTaskLifecycleStatus::Failed,
                Some(RuntimeExecutionOutcome {
                    compile: Some(RuntimeStageOutcome {
                        stage_name: "compile".to_owned(),
                        status: RuntimeStageStatus::Succeeded,
                        exit_code: Some(0),
                        signal: None,
                        failure_kind: None,
                        duration_ms: 3_000,
                        memory_used_kb: 65_536,
                        stdout_size_bytes: 0,
                        stderr_size_bytes: 0,
                        stdout_path: String::new(),
                        stderr_path: String::new(),
                        stdout_excerpt: String::new(),
                        stderr_excerpt: String::new(),
                    }),
                    judge_compile: None,
                    cases: vec![
                        RuntimeCaseOutcome {
                            case_no: 1,
                            score: 40,
                            status: RuntimeCaseFinalStatus::Accepted,
                            exit_code: Some(0),
                            duration_ms: 8,
                            memory_used_kb: 12_000,
                            stdout_path: String::new(),
                            stderr_path: String::new(),
                            stdout_excerpt: "ok".to_owned(),
                            stderr_excerpt: String::new(),
                        },
                        RuntimeCaseOutcome {
                            case_no: 2,
                            score: 60,
                            status: RuntimeCaseFinalStatus::WrongAnswer,
                            exit_code: Some(1),
                            duration_ms: 12,
                            memory_used_kb: 16_384,
                            stdout_path: String::new(),
                            stderr_path: String::new(),
                            stdout_excerpt: "5".to_owned(),
                            stderr_excerpt: String::new(),
                        },
                    ],
                    final_status: RuntimeTaskLifecycleStatus::Failed,
                }),
                "runtime execution failed",
            ))
            .await
            .expect("runtime event should be applied");

        let detail = service
            .get_submission_detail(&submission.submission_id.0)
            .await
            .expect("detail should exist");
        let result = detail.result.expect("result should be present");

        assert_eq!(detail.submission.status, SubmissionStatus::WrongAnswer);
        assert_eq!(result.overall_status, SubmissionStatus::WrongAnswer);
        assert_eq!(result.case_results.len(), 2);
        assert_eq!(result.time_used_ms, 20);
        assert_eq!(result.memory_used_kb, 16_384);
        assert_eq!(
            result.case_results[0].status,
            crate::domain::SubmissionCaseStatus::Accepted
        );
        assert_eq!(
            result.case_results[1].status,
            crate::domain::SubmissionCaseStatus::WrongAnswer
        );
        assert_eq!(result.case_results[1].memory_used_kb, 16_384);
        assert_eq!(result.case_results[1].score, 60);
    }

    #[tokio::test]
    async fn build_runtime_task_sets_queue_metadata_for_fast_lane() {
        let service = OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        );

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("two-sum"),
                user_id: UserId::from("u-1"),
                language: "cpp".to_owned(),
                source_code: "int main() { return 0; }".to_owned(),
            })
            .await
            .expect("submission should be created");

        let task = service
            .build_runtime_task(&submission.submission_id.0)
            .await
            .expect("runtime task should be built");

        assert_eq!(task.source_domain, "oj");
        assert_eq!(task.source_entity_id, submission.submission_id.0);
        assert_eq!(task.queue, "oj_judge");
        assert_eq!(task.lane, "fast");
    }

    #[tokio::test]
    async fn build_runtime_task_uses_special_lane_for_spj() {
        let mut limits = BTreeMap::new();
        limits.insert(
            "cpp".to_owned(),
            ProblemLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
        );

        let problem = Problem {
            problem_id: ProblemId::from("perm-spj"),
            title: "perm".to_owned(),
            slug: "perm".to_owned(),
            judge_mode: JudgeMode::Acm,
            sandbox_kind: SandboxKind::Nsjail,
            statement_md: "demo".to_owned(),
            supported_languages: vec!["cpp".to_owned()],
            limits,
            testcases: vec![Testcase {
                case_no: 1,
                input: "1\n".to_owned(),
                expected_output: "1\n".to_owned(),
                is_sample: true,
                score: 100,
            }],
            judge_config: Some(JudgeConfig {
                judge_method: JudgeMethod::Spj,
                validator: None,
                spj: Some(SpjConfig {
                    language: "cpp".to_owned(),
                    source_code: "int main() { return 0; }".to_owned(),
                }),
                function_signature: None,
            }),
            easy_config: None,
        };

        let problem_repo = Arc::new(InMemoryProblemRepository {
            problems: RwLock::new(vec![problem]),
        });
        let service = OjService::new(
            problem_repo,
            Arc::new(InMemorySubmissionRepository::default()),
        );

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("perm-spj"),
                user_id: UserId::from("u-1"),
                language: "cpp".to_owned(),
                source_code: "int main() { return 0; }".to_owned(),
            })
            .await
            .expect("submission should be created");

        let task = service
            .build_runtime_task(&submission.submission_id.0)
            .await
            .expect("runtime task should be built");

        assert_eq!(task.lane, "special");
    }

    fn base_problem(judge_config: Option<JudgeConfig>) -> Problem {
        let mut limits = BTreeMap::new();
        limits.insert(
            "cpp".to_owned(),
            ProblemLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
        );

        Problem {
            problem_id: ProblemId::from("p-1"),
            title: "demo".to_owned(),
            slug: "demo".to_owned(),
            judge_mode: JudgeMode::Acm,
            sandbox_kind: SandboxKind::Nsjail,
            statement_md: "demo".to_owned(),
            supported_languages: vec!["cpp".to_owned()],
            limits,
            testcases: vec![Testcase {
                case_no: 1,
                input: "1\n".to_owned(),
                expected_output: "1\n".to_owned(),
                is_sample: true,
                score: 100,
            }],
            judge_config,
            easy_config: None,
        }
    }

    fn runtime_event(
        submission_id: &str,
        status: RuntimeTaskLifecycleStatus,
        outcome: Option<RuntimeExecutionOutcome>,
        message: &str,
    ) -> RuntimeTaskEvent {
        RuntimeTaskEvent {
            task_id: "task-1".to_owned(),
            source_domain: "oj".to_owned(),
            queue: "oj_judge".to_owned(),
            lane: "fast".to_owned(),
            attempt: 1,
            submission_id: Some(submission_id.to_owned()),
            problem_id: Some("p-1".to_owned()),
            user_id: Some("u-1".to_owned()),
            language: Some("cpp".to_owned()),
            status,
            message: message.to_owned(),
            execution_id: Some("rt-1".to_owned()),
            outcome,
        }
    }

    #[test]
    fn validate_problem_rejects_wasm_with_spj() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Spj,
            validator: None,
            spj: Some(SpjConfig {
                language: "cpp".to_owned(),
                source_code: "int main() { return 0; }".to_owned(),
            }),
            function_signature: None,
        }));
        let problem = Problem {
            sandbox_kind: SandboxKind::Wasm,
            ..problem
        };

        let error = validate_problem(&problem).expect_err("wasm + spj should be rejected");
        assert!(error
            .to_string()
            .contains("wasm sandbox currently supports validator-based judging only"));
    }

    #[test]
    fn validate_problem_rejects_nsjail_wasm_with_spj() {
        let problem = base_problem(Some(JudgeConfig {
            judge_method: JudgeMethod::Spj,
            validator: None,
            spj: Some(SpjConfig {
                language: "cpp".to_owned(),
                source_code: "int main() { return 0; }".to_owned(),
            }),
            function_signature: None,
        }));
        let problem = Problem {
            sandbox_kind: SandboxKind::NsjailWasm,
            ..problem
        };

        let error = validate_problem(&problem).expect_err("nsjail_wasm + spj should be rejected");
        assert!(error
            .to_string()
            .contains("wasm sandbox currently supports validator-based judging only"));
    }

    #[tokio::test]
    async fn build_runtime_task_carries_wasm_sandbox_kind() {
        let mut problem = base_problem(None);
        problem.problem_id = ProblemId::from("wasm-rust");
        problem.slug = "wasm-rust".to_owned();
        problem.supported_languages = vec!["rust".to_owned()];
        problem.limits = BTreeMap::from([(
            "rust".to_owned(),
            ProblemLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
        )]);
        problem.sandbox_kind = SandboxKind::Wasm;

        let problem_repo = Arc::new(InMemoryProblemRepository {
            problems: RwLock::new(vec![problem]),
        });
        let service = OjService::new(
            problem_repo,
            Arc::new(InMemorySubmissionRepository::default()),
        );

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("wasm-rust"),
                user_id: UserId::from("u-1"),
                language: "rust".to_owned(),
                source_code: "fn main() { println!(\"3\"); }".to_owned(),
            })
            .await
            .expect("submission should be created");

        let task = service
            .build_runtime_task(&submission.submission_id.0)
            .await
            .expect("runtime task should be built");

        let payload = match task.payload {
            RuntimeTaskPayload::OjJudge(payload) => payload,
        };
        assert_eq!(payload.sandbox_kind, RuntimeSandboxKind::Wasm);
    }

    #[tokio::test]
    async fn build_runtime_task_carries_nsjail_wasm_sandbox_kind() {
        let mut problem = base_problem(None);
        problem.problem_id = ProblemId::from("nsjail-wasm-rust");
        problem.slug = "nsjail-wasm-rust".to_owned();
        problem.supported_languages = vec!["rust".to_owned()];
        problem.limits = BTreeMap::from([(
            "rust".to_owned(),
            ProblemLimits {
                time_limit_ms: 1000,
                memory_limit_kb: 262144,
            },
        )]);
        problem.sandbox_kind = SandboxKind::NsjailWasm;

        let problem_repo = Arc::new(InMemoryProblemRepository {
            problems: RwLock::new(vec![problem]),
        });
        let service = OjService::new(
            problem_repo,
            Arc::new(InMemorySubmissionRepository::default()),
        );

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("nsjail-wasm-rust"),
                user_id: UserId::from("u-1"),
                language: "rust".to_owned(),
                source_code: "fn main() { println!(\"3\"); }".to_owned(),
            })
            .await
            .expect("submission should be created");

        let task = service
            .build_runtime_task(&submission.submission_id.0)
            .await
            .expect("runtime task should be built");

        let payload = match task.payload {
            RuntimeTaskPayload::OjJudge(payload) => payload,
        };
        assert_eq!(payload.sandbox_kind, RuntimeSandboxKind::NsjailWasm);
    }

    fn runtime_case(
        case_no: u32,
        score: u32,
        status: RuntimeCaseFinalStatus,
    ) -> RuntimeCaseOutcome {
        RuntimeCaseOutcome {
            case_no,
            score,
            status,
            exit_code: Some(0),
            duration_ms: 10,
            memory_used_kb: 8_192,
            stdout_path: String::new(),
            stderr_path: String::new(),
            stdout_excerpt: String::new(),
            stderr_excerpt: String::new(),
        }
    }
}

fn map_runtime_judge_config(config: crate::domain::JudgeConfig) -> RuntimeJudgeConfig {
    RuntimeJudgeConfig {
        judge_method: match config.judge_method {
            JudgeMethod::Validator => RuntimeJudgeMethod::Validator,
            JudgeMethod::Spj => RuntimeJudgeMethod::Spj,
        },
        validator: config.validator.map(|validator| RuntimeValidatorConfig {
            ignore_whitespace: validator.ignore_whitespace,
            ignore_case: validator.ignore_case,
            is_unordered: validator.is_unordered,
            is_token_mode: validator.is_token_mode,
            is_float: validator.is_float,
            float_epsilon: validator.float_epsilon,
        }),
        spj: config.spj.map(|spj| RuntimeSpjConfig {
            language: spj.language,
            source_code: spj.source_code,
        }),
        function_signature: config
            .function_signature
            .map(|signature| RuntimeFunctionSignature {
                function_name: signature.function_name,
                return_type: signature.return_type,
                params: signature
                    .params
                    .into_iter()
                    .map(|param| RuntimeFunctionParameter {
                        name: param.name,
                        ty: param.ty,
                    })
                    .collect(),
            }),
    }
}

struct EasyJudgeEvaluation {
    status: SubmissionStatus,
    score: u32,
    max_score: u32,
    message: Option<String>,
}

fn judge_easy_answer(
    config: &EasyProblemConfig,
    answer: &EasyAnswer,
) -> AppResult<EasyJudgeEvaluation> {
    let max_score = 100;
    let accepted = match config.question_type {
        EasyQuestionType::TrueFalse | EasyQuestionType::SingleChoice => {
            normalize_easy_answer(answer) == normalize_easy_answer(&config.standard_answer)
        }
        EasyQuestionType::MultipleChoice => {
            normalize_multi_answer(answer)? == normalize_multi_answer(&config.standard_answer)?
        }
    };

    Ok(if accepted {
        EasyJudgeEvaluation {
            status: SubmissionStatus::Accepted,
            score: max_score,
            max_score,
            message: Some("accepted".to_owned()),
        }
    } else {
        EasyJudgeEvaluation {
            status: SubmissionStatus::WrongAnswer,
            score: 0,
            max_score,
            message: Some("wrong_answer".to_owned()),
        }
    })
}

fn normalize_easy_answer(answer: &EasyAnswer) -> String {
    match answer {
        EasyAnswer::Text(text) => text.trim().to_uppercase(),
        EasyAnswer::Options(values) => values.join("").trim().to_uppercase(),
    }
}

fn normalize_multi_answer(answer: &EasyAnswer) -> AppResult<Vec<String>> {
    let mut values = match answer {
        EasyAnswer::Text(text) => text
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .map(|ch| ch.to_string())
            .collect::<Vec<_>>(),
        EasyAnswer::Options(items) => items.clone(),
    };

    values
        .iter_mut()
        .for_each(|item| *item = item.trim().to_uppercase());
    values.retain(|item| !item.is_empty());
    values.sort();
    values.dedup();

    if values.is_empty() {
        return Err(AppError::BadRequest(
            "multiple_choice answer cannot be empty".to_owned(),
        ));
    }

    Ok(values)
}
