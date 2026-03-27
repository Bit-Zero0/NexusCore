use std::collections::BTreeMap;

use async_trait::async_trait;
use nexus_runtime::{RuntimeCaseFinalStatus, RuntimeTaskEvent};
use sqlx::{PgPool, Row};

use crate::{
    domain::{
        EasySubmissionDraft, Problem, ProblemDetail, ProblemLimits, ProblemSummary, SandboxKind,
        SubmissionCaseResult, SubmissionCaseStatus, SubmissionDetail, SubmissionDraft,
        SubmissionRecord, SubmissionResult, SubmissionStatus, Testcase,
    },
    repository::{ProblemRepository, SubmissionRepository},
    JudgeMode,
};
use nexus_shared::{AppError, AppResult, ProblemId, SubmissionId};

pub struct PgProblemRepository {
    pool: PgPool,
}

impl PgProblemRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProblemRepository for PgProblemRepository {
    async fn list_summaries(&self) -> AppResult<Vec<ProblemSummary>> {
        let rows = sqlx::query(
            r#"
            select problem_id, title, slug, judge_mode
            from oj.problems
            order by created_at desc
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(ProblemSummary {
                problem_id: ProblemId(
                    row.try_get("problem_id")
                        .map_err(|err| AppError::Database(err.to_string()))?,
                ),
                title: row
                    .try_get("title")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                slug: row
                    .try_get("slug")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                judge_mode: parse_judge_mode(
                    &row.try_get::<String, _>("judge_mode")
                        .map_err(|err| AppError::Database(err.to_string()))?,
                )?,
            });
        }

        Ok(items)
    }

    async fn find_detail(&self, problem_id: &str) -> AppResult<Option<ProblemDetail>> {
        let problem_row = sqlx::query(
            r#"
            select problem_id, title, slug, judge_mode, sandbox_kind, statement_md, judge_config, easy_config
            from oj.problems
            where problem_id = $1
            "#,
        )
        .bind(problem_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let Some(problem_row) = problem_row else {
            return Ok(None);
        };

        let limit_rows = sqlx::query(
            r#"
            select language, time_limit_ms, memory_limit_kb
            from oj.problem_limits
            where problem_id = $1
            order by language asc
            "#,
        )
        .bind(problem_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let testcase_rows = sqlx::query(
            r#"
            select case_no, input_data, expected_output, is_sample, score
            from oj.problem_testcases
            where problem_id = $1
            order by case_no asc
            "#,
        )
        .bind(problem_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let mut supported_languages = Vec::with_capacity(limit_rows.len());
        let mut limits = BTreeMap::new();
        let mut testcases = Vec::with_capacity(testcase_rows.len());

        for row in limit_rows {
            let language: String = row
                .try_get("language")
                .map_err(|err| AppError::Database(err.to_string()))?;
            supported_languages.push(language.clone());
            limits.insert(
                language,
                ProblemLimits {
                    time_limit_ms: row
                        .try_get::<i64, _>("time_limit_ms")
                        .map_err(|err| AppError::Database(err.to_string()))?
                        as u64,
                    memory_limit_kb: row
                        .try_get::<i64, _>("memory_limit_kb")
                        .map_err(|err| AppError::Database(err.to_string()))?
                        as u64,
                },
            );
        }

        for row in testcase_rows {
            testcases.push(Testcase {
                case_no: row
                    .try_get::<i32, _>("case_no")
                    .map_err(|err| AppError::Database(err.to_string()))?
                    as u32,
                input: row
                    .try_get("input_data")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                expected_output: row
                    .try_get("expected_output")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                is_sample: row
                    .try_get("is_sample")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                score: row
                    .try_get::<i32, _>("score")
                    .map_err(|err| AppError::Database(err.to_string()))?
                    as u32,
            });
        }

        Ok(Some(ProblemDetail {
            problem: Problem {
                problem_id: ProblemId(
                    problem_row
                        .try_get("problem_id")
                        .map_err(|err| AppError::Database(err.to_string()))?,
                ),
                title: problem_row
                    .try_get("title")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                slug: problem_row
                    .try_get("slug")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                judge_mode: parse_judge_mode(
                    &problem_row
                        .try_get::<String, _>("judge_mode")
                        .map_err(|err| AppError::Database(err.to_string()))?,
                )?,
                sandbox_kind: parse_sandbox_kind(
                    &problem_row
                        .try_get::<String, _>("sandbox_kind")
                        .map_err(|err| AppError::Database(err.to_string()))?,
                )?,
                statement_md: problem_row
                    .try_get("statement_md")
                    .map_err(|err| AppError::Database(err.to_string()))?,
                supported_languages,
                limits,
                testcases,
                judge_config: problem_row
                    .try_get::<Option<serde_json::Value>, _>("judge_config")
                    .map_err(|err| AppError::Database(err.to_string()))?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|err| AppError::Database(err.to_string()))?,
                easy_config: problem_row
                    .try_get::<Option<serde_json::Value>, _>("easy_config")
                    .map_err(|err| AppError::Database(err.to_string()))?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|err| AppError::Database(err.to_string()))?,
            },
        }))
    }

    async fn save(&self, problem: Problem) -> AppResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        sqlx::query(
            r#"
            insert into oj.problems (
                problem_id,
                title,
                slug,
                judge_mode,
                sandbox_kind,
                statement_md,
                judge_config,
                easy_config
            )
            values ($1, $2, $3, $4, $5, $6, $7, $8)
            on conflict (problem_id) do update
            set title = excluded.title,
                slug = excluded.slug,
                judge_mode = excluded.judge_mode,
                sandbox_kind = excluded.sandbox_kind,
                statement_md = excluded.statement_md,
                judge_config = excluded.judge_config,
                easy_config = excluded.easy_config,
                updated_at = now()
            "#,
        )
        .bind(&problem.problem_id.0)
        .bind(&problem.title)
        .bind(&problem.slug)
        .bind(format_judge_mode(&problem.judge_mode))
        .bind(format_sandbox_kind(problem.sandbox_kind))
        .bind(&problem.statement_md)
        .bind(
            problem
                .judge_config
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|err| AppError::Database(err.to_string()))?,
        )
        .bind(
            problem
                .easy_config
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|err| AppError::Database(err.to_string()))?,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        sqlx::query("delete from oj.problem_limits where problem_id = $1")
            .bind(&problem.problem_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        sqlx::query("delete from oj.problem_testcases where problem_id = $1")
            .bind(&problem.problem_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        for (language, limits) in &problem.limits {
            sqlx::query(
                r#"
                insert into oj.problem_limits (problem_id, language, time_limit_ms, memory_limit_kb)
                values ($1, $2, $3, $4)
                "#,
            )
            .bind(&problem.problem_id.0)
            .bind(language)
            .bind(limits.time_limit_ms as i64)
            .bind(limits.memory_limit_kb as i64)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;
        }

        for testcase in &problem.testcases {
            sqlx::query(
                r#"
                insert into oj.problem_testcases (
                    problem_id,
                    case_no,
                    input_data,
                    expected_output,
                    is_sample,
                    score
                ) values ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(&problem.problem_id.0)
            .bind(testcase.case_no as i32)
            .bind(&testcase.input)
            .bind(&testcase.expected_output)
            .bind(testcase.is_sample)
            .bind(testcase.score as i32)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(())
    }
}

pub struct PgSubmissionRepository {
    pool: PgPool,
}

impl PgSubmissionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SubmissionRepository for PgSubmissionRepository {
    async fn validate_submission(&self, draft: &SubmissionDraft) -> AppResult<()> {
        if draft.source_code.trim().is_empty() {
            return Err(AppError::BadRequest(
                "submission source code cannot be empty".to_owned(),
            ));
        }

        let count = sqlx::query_scalar::<_, i64>(
            r#"
            select count(1)
            from oj.problems
            where problem_id = $1
            "#,
        )
        .bind(&draft.problem_id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        if count == 0 {
            return Err(AppError::NotFound(format!(
                "problem not found: {}",
                draft.problem_id.0
            )));
        }

        Ok(())
    }

    async fn list_submissions(&self) -> AppResult<Vec<SubmissionRecord>> {
        let rows = sqlx::query(
            r#"
            select submission_id, problem_id, user_id, language, status, score, max_score, result_message
            from oj.submissions
            order by created_at desc
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(map_submission_record(&row)?);
        }

        Ok(items)
    }

    async fn find_submission(&self, submission_id: &str) -> AppResult<Option<SubmissionDetail>> {
        let row = sqlx::query(
            r#"
            select submission_id, problem_id, user_id, language, status, score, max_score, result_message, source_code, execution_summary
            from oj.submissions
            where submission_id = $1
            "#,
        )
        .bind(submission_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let Some(row) = row else {
            return Ok(None);
        };

        let submission = map_submission_record(&row)?;
        let case_rows = sqlx::query(
            r#"
                    select
                        scr.case_no,
                        scr.score,
                        scr.status,
                        scr.duration_ms,
                        scr.memory_used_kb,
                        scr.stdout_excerpt,
                        scr.stderr_excerpt,
                        pt.expected_output
            from oj.submission_case_results scr
            join oj.submissions s on s.submission_id = scr.submission_id
            left join oj.problem_testcases pt on pt.problem_id = s.problem_id and pt.case_no = scr.case_no
            where scr.submission_id = $1
            order by scr.case_no asc
            "#,
        )
        .bind(submission_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        let case_results = case_rows
            .iter()
            .map(map_submission_case_result)
            .collect::<AppResult<Vec<_>>>()?;
        let execution_summary = row
            .try_get::<Option<serde_json::Value>, _>("execution_summary")
            .map_err(|err| AppError::Database(err.to_string()))?
            .map(serde_json::from_value)
            .transpose()
            .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(Some(SubmissionDetail {
            submission: submission.clone(),
            source_code: row
                .try_get("source_code")
                .map_err(|err| AppError::Database(err.to_string()))?,
            result: Some(build_submission_result(
                &submission,
                execution_summary.as_ref(),
                case_results,
            )),
        }))
    }

    async fn create_submission(&self, draft: SubmissionDraft) -> AppResult<SubmissionRecord> {
        let submission_id = format!("sub-{}", ulid::Ulid::new());
        sqlx::query(
            r#"
            insert into oj.submissions (
                submission_id,
                problem_id,
                user_id,
                language,
                source_code,
                status
            ) values ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(&submission_id)
        .bind(&draft.problem_id.0)
        .bind(&draft.user_id.0)
        .bind(&draft.language)
        .bind(&draft.source_code)
        .bind("pending")
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(SubmissionRecord {
            submission_id: SubmissionId::from(submission_id),
            problem_id: draft.problem_id,
            user_id: draft.user_id,
            language: draft.language,
            status: SubmissionStatus::Pending,
            score: 0,
            max_score: 0,
            message: None,
        })
    }

    async fn apply_runtime_event(&self, event: &RuntimeTaskEvent) -> AppResult<()> {
        let Some(submission_id) = &event.submission_id else {
            return Ok(());
        };

        let (status, score, max_score, message) =
            crate::application::derive_submission_projection(event);
        let summary =
            serde_json::to_value(event).map_err(|err| AppError::Database(err.to_string()))?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        sqlx::query(
            r#"
            update oj.submissions
            set status = $2,
                score = $3,
                max_score = $4,
                result_message = $5,
                execution_summary = $6,
                updated_at = now()
            where submission_id = $1
            "#,
        )
        .bind(submission_id)
        .bind(format_submission_status(&status))
        .bind(score as i32)
        .bind(max_score as i32)
        .bind(message)
        .bind(summary)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        if let Some(outcome) = &event.outcome {
            sqlx::query("delete from oj.submission_case_results where submission_id = $1")
                .bind(submission_id)
                .execute(&mut *tx)
                .await
                .map_err(|err| AppError::Database(err.to_string()))?;

            for case in &outcome.cases {
                sqlx::query(
                    r#"
                    insert into oj.submission_case_results (
                        submission_id,
                        case_no,
                        score,
                        status,
                        exit_code,
                        duration_ms,
                        memory_used_kb,
                        stdout_path,
                        stderr_path,
                        stdout_excerpt,
                        stderr_excerpt
                    ) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                    "#,
                )
                .bind(submission_id)
                .bind(case.case_no as i32)
                .bind(case.score as i32)
                .bind(format_case_status(&case.status))
                .bind(case.exit_code)
                .bind(case.duration_ms as i64)
                .bind(case.memory_used_kb as i64)
                .bind(&case.stdout_path)
                .bind(&case.stderr_path)
                .bind(&case.stdout_excerpt)
                .bind(&case.stderr_excerpt)
                .execute(&mut *tx)
                .await
                .map_err(|err| AppError::Database(err.to_string()))?;
            }
        }

        tx.commit()
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(())
    }

    async fn create_direct_submission(
        &self,
        draft: EasySubmissionDraft,
        status: SubmissionStatus,
        score: u32,
        max_score: u32,
        message: Option<String>,
        stored_answer: String,
    ) -> AppResult<SubmissionRecord> {
        let submission_id = format!("sub-{}", ulid::Ulid::new());
        sqlx::query(
            r#"
            insert into oj.submissions (
                submission_id,
                problem_id,
                user_id,
                language,
                source_code,
                status,
                score,
                max_score,
                result_message
            ) values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(&submission_id)
        .bind(&draft.problem_id.0)
        .bind(&draft.user_id.0)
        .bind("easy_judge")
        .bind(&stored_answer)
        .bind(format_submission_status(&status))
        .bind(score as i32)
        .bind(max_score as i32)
        .bind(&message)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(SubmissionRecord {
            submission_id: SubmissionId::from(submission_id),
            problem_id: draft.problem_id,
            user_id: draft.user_id,
            language: "easy_judge".to_owned(),
            status,
            score,
            max_score,
            message,
        })
    }
}

fn parse_judge_mode(value: &str) -> AppResult<JudgeMode> {
    match value {
        "acm" => Ok(JudgeMode::Acm),
        "functional" => Ok(JudgeMode::Functional),
        "easy_judge" => Ok(JudgeMode::EasyJudge),
        _ => Err(AppError::BadRequest(format!(
            "unsupported judge mode from storage: {value}"
        ))),
    }
}

fn format_judge_mode(mode: &JudgeMode) -> &'static str {
    match mode {
        JudgeMode::Acm => "acm",
        JudgeMode::Functional => "functional",
        JudgeMode::EasyJudge => "easy_judge",
    }
}

fn parse_sandbox_kind(value: &str) -> AppResult<SandboxKind> {
    match value {
        "nsjail" => Ok(SandboxKind::Nsjail),
        "wasm" => Ok(SandboxKind::Wasm),
        "nsjail_wasm" => Ok(SandboxKind::NsjailWasm),
        _ => Err(AppError::BadRequest(format!(
            "unsupported sandbox kind from storage: {value}"
        ))),
    }
}

fn format_sandbox_kind(kind: SandboxKind) -> &'static str {
    match kind {
        SandboxKind::Nsjail => "nsjail",
        SandboxKind::Wasm => "wasm",
        SandboxKind::NsjailWasm => "nsjail_wasm",
    }
}

fn format_submission_status(status: &SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::Pending => "pending",
        SubmissionStatus::Queued => "queued",
        SubmissionStatus::Running => "running",
        SubmissionStatus::Accepted => "accepted",
        SubmissionStatus::WrongAnswer => "wrong_answer",
        SubmissionStatus::CompileError => "compile_error",
        SubmissionStatus::TimeLimitExceeded => "time_limit_exceeded",
        SubmissionStatus::MemoryLimitExceeded => "memory_limit_exceeded",
        SubmissionStatus::OutputLimitExceeded => "output_limit_exceeded",
        SubmissionStatus::SecurityViolation => "security_violation",
        SubmissionStatus::RuntimeError => "runtime_error",
    }
}

fn parse_submission_status(value: &str) -> AppResult<SubmissionStatus> {
    match value {
        "pending" => Ok(SubmissionStatus::Pending),
        "queued" => Ok(SubmissionStatus::Queued),
        "running" => Ok(SubmissionStatus::Running),
        "accepted" => Ok(SubmissionStatus::Accepted),
        "wrong_answer" => Ok(SubmissionStatus::WrongAnswer),
        "compile_error" => Ok(SubmissionStatus::CompileError),
        "time_limit_exceeded" => Ok(SubmissionStatus::TimeLimitExceeded),
        "memory_limit_exceeded" => Ok(SubmissionStatus::MemoryLimitExceeded),
        "output_limit_exceeded" => Ok(SubmissionStatus::OutputLimitExceeded),
        "security_violation" => Ok(SubmissionStatus::SecurityViolation),
        "runtime_error" => Ok(SubmissionStatus::RuntimeError),
        _ => Err(AppError::BadRequest(format!(
            "unsupported submission status from storage: {value}"
        ))),
    }
}

fn map_submission_record(row: &sqlx::postgres::PgRow) -> AppResult<SubmissionRecord> {
    Ok(SubmissionRecord {
        submission_id: SubmissionId(
            row.try_get("submission_id")
                .map_err(|err| AppError::Database(err.to_string()))?,
        ),
        problem_id: ProblemId(
            row.try_get("problem_id")
                .map_err(|err| AppError::Database(err.to_string()))?,
        ),
        user_id: nexus_shared::UserId(
            row.try_get("user_id")
                .map_err(|err| AppError::Database(err.to_string()))?,
        ),
        language: row
            .try_get("language")
            .map_err(|err| AppError::Database(err.to_string()))?,
        status: parse_submission_status(
            &row.try_get::<String, _>("status")
                .map_err(|err| AppError::Database(err.to_string()))?,
        )?,
        score: row
            .try_get::<i32, _>("score")
            .map_err(|err| AppError::Database(err.to_string()))? as u32,
        max_score: row
            .try_get::<i32, _>("max_score")
            .map_err(|err| AppError::Database(err.to_string()))? as u32,
        message: row
            .try_get("result_message")
            .map_err(|err| AppError::Database(err.to_string()))?,
    })
}

fn format_case_status(status: &RuntimeCaseFinalStatus) -> &'static str {
    match status {
        RuntimeCaseFinalStatus::Accepted => "accepted",
        RuntimeCaseFinalStatus::WrongAnswer => "wrong_answer",
        RuntimeCaseFinalStatus::TimeLimitExceeded => "time_limit_exceeded",
        RuntimeCaseFinalStatus::MemoryLimitExceeded => "memory_limit_exceeded",
        RuntimeCaseFinalStatus::OutputLimitExceeded => "output_limit_exceeded",
        RuntimeCaseFinalStatus::SecurityViolation => "security_violation",
        RuntimeCaseFinalStatus::RuntimeError => "runtime_error",
    }
}

fn map_submission_case_result(row: &sqlx::postgres::PgRow) -> AppResult<SubmissionCaseResult> {
    let stderr_excerpt: String = row
        .try_get("stderr_excerpt")
        .map_err(|err| AppError::Database(err.to_string()))?;
    Ok(SubmissionCaseResult {
        case_no: row
            .try_get::<i32, _>("case_no")
            .map_err(|err| AppError::Database(err.to_string()))? as u32,
        status: parse_submission_case_status(
            &row.try_get::<String, _>("status")
                .map_err(|err| AppError::Database(err.to_string()))?,
        )?,
        score: row
            .try_get::<i32, _>("score")
            .map_err(|err| AppError::Database(err.to_string()))? as u32,
        time_used_ms: row
            .try_get::<i64, _>("duration_ms")
            .map_err(|err| AppError::Database(err.to_string()))? as u64,
        memory_used_kb: row.try_get::<i64, _>("memory_used_kb").unwrap_or_default() as u64,
        actual_output: row
            .try_get("stdout_excerpt")
            .map_err(|err| AppError::Database(err.to_string()))?,
        expected_output_snapshot: row
            .try_get::<Option<String>, _>("expected_output")
            .map_err(|err| AppError::Database(err.to_string()))?
            .unwrap_or_default(),
        message: if stderr_excerpt.is_empty() {
            None
        } else {
            Some(stderr_excerpt)
        },
    })
}

fn parse_submission_case_status(value: &str) -> AppResult<SubmissionCaseStatus> {
    match value {
        "accepted" => Ok(SubmissionCaseStatus::Accepted),
        "wrong_answer" => Ok(SubmissionCaseStatus::WrongAnswer),
        "time_limit_exceeded" => Ok(SubmissionCaseStatus::TimeLimitExceeded),
        "memory_limit_exceeded" => Ok(SubmissionCaseStatus::MemoryLimitExceeded),
        "output_limit_exceeded" => Ok(SubmissionCaseStatus::OutputLimitExceeded),
        "security_violation" => Ok(SubmissionCaseStatus::SecurityViolation),
        "runtime_error" => Ok(SubmissionCaseStatus::RuntimeError),
        _ => Err(AppError::BadRequest(format!(
            "unsupported submission case status from storage: {value}"
        ))),
    }
}

fn build_submission_result(
    submission: &SubmissionRecord,
    execution_summary: Option<&RuntimeTaskEvent>,
    case_results: Vec<SubmissionCaseResult>,
) -> SubmissionResult {
    let compile_output = execution_summary.and_then(extract_compile_output);
    let runtime_output = execution_summary
        .and_then(extract_runtime_output)
        .or_else(|| {
            case_results.iter().find_map(|case| {
                case.message.clone().or_else(|| {
                    if case.actual_output.is_empty() {
                        None
                    } else {
                        Some(case.actual_output.clone())
                    }
                })
            })
        });
    let time_used_ms = execution_summary
        .map(total_case_duration_ms)
        .unwrap_or_else(|| case_results.iter().map(|case| case.time_used_ms).sum());

    SubmissionResult {
        submission_id: submission.submission_id.clone(),
        overall_status: submission.status.clone(),
        compile_output,
        runtime_output,
        compile_time_ms: execution_summary.map(extract_compile_time_ms).unwrap_or(0),
        judge_compile_time_ms: execution_summary
            .map(extract_judge_compile_time_ms)
            .unwrap_or(0),
        run_time_ms: time_used_ms,
        time_used_ms,
        memory_used_kb: case_results
            .iter()
            .map(|case| case.memory_used_kb)
            .max()
            .unwrap_or(0),
        judge_summary: execution_summary
            .map(|event| event.message.clone())
            .or_else(|| submission.message.clone()),
        case_results,
    }
}

fn extract_compile_output(summary: &RuntimeTaskEvent) -> Option<String> {
    let outcome = summary.outcome.as_ref()?;
    if let Some(stage) = &outcome.judge_compile {
        if !stage.stderr_excerpt.is_empty() {
            return Some(stage.stderr_excerpt.clone());
        }
    }
    if let Some(stage) = &outcome.compile {
        if !stage.stderr_excerpt.is_empty() {
            return Some(stage.stderr_excerpt.clone());
        }
    }
    None
}

fn extract_runtime_output(summary: &RuntimeTaskEvent) -> Option<String> {
    let outcome = summary.outcome.as_ref()?;
    outcome.cases.iter().find_map(|case| {
        if !case.stderr_excerpt.is_empty() {
            Some(case.stderr_excerpt.clone())
        } else if !case.stdout_excerpt.is_empty() {
            Some(case.stdout_excerpt.clone())
        } else {
            None
        }
    })
}

fn extract_compile_time_ms(summary: &RuntimeTaskEvent) -> u64 {
    summary
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.compile.as_ref())
        .map(|stage| stage.duration_ms as u64)
        .unwrap_or(0)
}

fn extract_judge_compile_time_ms(summary: &RuntimeTaskEvent) -> u64 {
    summary
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.judge_compile.as_ref())
        .map(|stage| stage.duration_ms as u64)
        .unwrap_or(0)
}

fn total_case_duration_ms(summary: &RuntimeTaskEvent) -> u64 {
    let Some(outcome) = summary.outcome.as_ref() else {
        return 0;
    };

    outcome
        .cases
        .iter()
        .map(|case| case.duration_ms as u64)
        .sum()
}
