mod application;
mod domain;
mod infrastructure;
mod language;
mod repository;

use std::sync::Arc;

pub use application::{InMemoryProblemRepository, InMemorySubmissionRepository, OjService};
use axum::{
    extract::Path,
    extract::State,
    routing::{get, post, put},
    Json, Router,
};
pub use domain::{
    EasyAnswer, EasyProblemConfig, EasyQuestionType, EasySubmissionDraft, JudgeMode, Problem,
    ProblemDetail, ProblemLimits, ProblemSummary, SubmissionCaseResult, SubmissionDetail,
    SubmissionDraft, SubmissionRecord, SubmissionResult,
};
pub use infrastructure::{PgProblemRepository, PgSubmissionRepository};
pub use language::{
    build_default_catalog, CodeTemplateRequest, LanguageCatalog, LanguageDescriptor, LanguageKey,
    LanguageSpec, SandboxProfile, SeccompPolicy,
};
use nexus_runtime::{RuntimeTask, RuntimeTaskService};
use nexus_shared::{AppError, AppResult};
pub use repository::{ProblemRepository, SubmissionRepository};

#[derive(Clone)]
struct OjState {
    catalog: Arc<LanguageCatalog>,
    service: Arc<OjService>,
    runtime_service: Option<Arc<RuntimeTaskService>>,
}

pub fn build_router(
    catalog: Arc<LanguageCatalog>,
    service: Arc<OjService>,
    runtime_service: Option<Arc<RuntimeTaskService>>,
) -> Router {
    Router::new()
        .route("/api/v1/oj/catalog/languages", get(list_languages))
        .route("/api/v1/oj/catalog/judge-modes", get(list_judge_modes))
        .route(
            "/api/v1/oj/catalog/templates/:language/:mode",
            get(get_template),
        )
        .route(
            "/api/v1/oj/problems",
            get(list_problems).post(create_problem),
        )
        .route("/api/v1/oj/problems/:problem_id", get(get_problem))
        .route("/api/v1/oj/problems/:problem_id", put(update_problem))
        .route(
            "/api/v1/oj/submissions",
            get(list_submissions).post(create_submission),
        )
        .route("/api/v1/oj/submissions/:submission_id", get(get_submission))
        .route(
            "/api/v1/oj/submissions/:submission_id/runtime-task",
            get(get_runtime_task),
        )
        .route(
            "/api/v1/oj/easy-judge/submissions",
            post(create_easy_submission),
        )
        .with_state(OjState {
            catalog,
            service,
            runtime_service,
        })
}

async fn list_languages(State(state): State<OjState>) -> Json<Vec<LanguageDescriptor>> {
    Json(state.catalog.descriptors())
}

async fn list_judge_modes() -> Json<Vec<JudgeMode>> {
    Json(vec![
        JudgeMode::Acm,
        JudgeMode::Functional,
        JudgeMode::EasyJudge,
    ])
}

async fn get_template(
    Path((language, mode)): Path<(String, String)>,
    State(state): State<OjState>,
) -> AppResult<Json<CodeTemplateRequest>> {
    let mode = JudgeMode::from_path(&mode)
        .ok_or_else(|| AppError::BadRequest(format!("unsupported judge mode: {mode}")))?;
    let template = state
        .catalog
        .template_for(&language, &mode)
        .ok_or_else(|| AppError::NotFound(format!("language not found: {language}")))?;

    Ok(Json(CodeTemplateRequest {
        language: language.clone(),
        judge_mode: mode,
        template,
    }))
}

async fn list_problems(State(state): State<OjState>) -> AppResult<Json<Vec<ProblemSummary>>> {
    let problems = state.service.list_problem_summaries().await?;
    Ok(Json(problems))
}

async fn get_problem(
    Path(problem_id): Path<String>,
    State(state): State<OjState>,
) -> AppResult<Json<ProblemDetail>> {
    let problem = state.service.get_problem_detail(&problem_id).await?;
    Ok(Json(problem))
}

async fn create_problem(
    State(state): State<OjState>,
    Json(problem): Json<Problem>,
) -> AppResult<Json<ProblemDetail>> {
    state.service.save_problem(problem.clone()).await?;
    Ok(Json(ProblemDetail { problem }))
}

async fn update_problem(
    Path(problem_id): Path<String>,
    State(state): State<OjState>,
    Json(problem): Json<Problem>,
) -> AppResult<Json<ProblemDetail>> {
    if problem.problem_id.0 != problem_id {
        return Err(AppError::BadRequest(
            "path problem_id does not match body problem_id".to_owned(),
        ));
    }

    state.service.save_problem(problem.clone()).await?;
    Ok(Json(ProblemDetail { problem }))
}

async fn create_submission(
    State(state): State<OjState>,
    Json(draft): Json<SubmissionDraft>,
) -> AppResult<Json<SubmissionRecord>> {
    let submission = state.service.create_submission(draft).await?;

    if let Some(runtime_service) = &state.runtime_service {
        let task = state
            .service
            .build_runtime_task(&submission.submission_id.0)
            .await?;
        runtime_service.schedule(task).await?;
    }

    Ok(Json(submission))
}

async fn list_submissions(State(state): State<OjState>) -> AppResult<Json<Vec<SubmissionRecord>>> {
    let submissions = state.service.list_submissions().await?;
    Ok(Json(submissions))
}

async fn get_submission(
    Path(submission_id): Path<String>,
    State(state): State<OjState>,
) -> AppResult<Json<SubmissionDetail>> {
    let submission = state.service.get_submission_detail(&submission_id).await?;
    Ok(Json(submission))
}

async fn get_runtime_task(
    Path(submission_id): Path<String>,
    State(state): State<OjState>,
) -> AppResult<Json<RuntimeTask>> {
    let task = state.service.build_runtime_task(&submission_id).await?;
    Ok(Json(task))
}

async fn create_easy_submission(
    State(state): State<OjState>,
    Json(draft): Json<EasySubmissionDraft>,
) -> AppResult<Json<SubmissionRecord>> {
    let submission = state.service.judge_easy_submission(draft).await?;
    Ok(Json(submission))
}

#[cfg(test)]
mod tests {
    use super::{
        build_default_catalog, build_router, InMemoryProblemRepository,
        InMemorySubmissionRepository, OjService,
    };
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use nexus_runtime::{
        build_default_runtime_catalog, InMemoryRuntimeTaskQueue, NoopRuntimeEventObserver,
        RuntimeCaseFinalStatus, RuntimeCaseOutcome, RuntimeExecutionOutcome, RuntimeSeccompMode,
        RuntimeSyscallArch, RuntimeSyscallFlavor, RuntimeTaskEvent, RuntimeTaskLifecycleStatus,
        RuntimeTaskService, RuntimeWorker,
    };
    use nexus_shared::{ProblemId, UserId};
    use serde_json::Value;
    use std::sync::Arc;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn submission_detail_route_returns_stable_result_shape() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service.clone(), None);

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("two-sum"),
                user_id: UserId::from("u-http"),
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
                    compile: None,
                    judge_compile: None,
                    cases: vec![
                        RuntimeCaseOutcome {
                            case_no: 1,
                            score: 40,
                            status: RuntimeCaseFinalStatus::Accepted,
                            exit_code: Some(0),
                            duration_ms: 8,
                            memory_used_kb: 12_288,
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

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/oj/submissions/{}",
                        submission.submission_id.0
                    ))
                    .body(Body::empty())
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(json["submission"]["status"], "wrong_answer");
        assert_eq!(json["result"]["overall_status"], "wrong_answer");
        assert_eq!(json["result"]["case_results"][0]["status"], "accepted");
        assert_eq!(json["result"]["case_results"][1]["status"], "wrong_answer");
        assert_eq!(json["result"]["case_results"][1]["score"], 60);
    }

    #[tokio::test]
    async fn runtime_task_route_returns_queue_lane_and_retry_policy() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service.clone(), None);

        let submission = service
            .create_submission(crate::domain::SubmissionDraft {
                problem_id: ProblemId::from("two-sum"),
                user_id: UserId::from("u-http"),
                language: "cpp".to_owned(),
                source_code: "int main() { return 0; }".to_owned(),
            })
            .await
            .expect("submission should be created");

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/oj/submissions/{}/runtime-task",
                        submission.submission_id.0
                    ))
                    .body(Body::empty())
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(json["source_domain"], "oj");
        assert_eq!(json["queue"], "oj_judge");
        assert_eq!(json["lane"], "fast");
        assert_eq!(json["retry_policy"]["max_attempts"], 3);
        assert_eq!(json["retry_policy"]["retry_delay_ms"], 1000);
    }

    #[tokio::test]
    async fn create_problem_route_returns_problem_detail_shape() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service, None);

        let body = serde_json::json!({
            "problem_id": "sum-http",
            "title": "sum",
            "slug": "sum-http",
            "judge_mode": "acm",
            "statement_md": "demo",
            "supported_languages": ["cpp"],
            "limits": {
                "cpp": {
                    "time_limit_ms": 1000,
                    "memory_limit_kb": 262144
                }
            },
            "testcases": [{
                "case_no": 1,
                "input": "1 2\n",
                "expected_output": "3\n",
                "is_sample": true,
                "score": 100
            }],
            "judge_config": null,
            "easy_config": null
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/oj/problems")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(json["problem"]["problem_id"], "sum-http");
        assert_eq!(json["problem"]["judge_mode"], "acm");
        assert_eq!(json["problem"]["limits"]["cpp"]["time_limit_ms"], 1000);
        assert_eq!(json["problem"]["testcases"][0]["score"], 100);
    }

    #[tokio::test]
    async fn create_submission_route_returns_submission_record_shape() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let runtime_service = Arc::new(RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                build_default_runtime_catalog(),
                "/tmp/nexuscode-runtime-test",
                "/usr/bin/nsjail",
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                RuntimeSyscallArch::X86_64,
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(NoopRuntimeEventObserver),
        ));
        let router = build_router(
            build_default_catalog(),
            service,
            Some(runtime_service.clone()),
        );

        let body = serde_json::json!({
            "problem_id": "two-sum",
            "user_id": "u-submit",
            "language": "cpp",
            "source_code": "int main() { return 0; }"
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/oj/submissions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(json["problem_id"], "two-sum");
        assert_eq!(json["user_id"], "u-submit");
        assert_eq!(json["language"], "cpp");
        assert_eq!(json["status"], "pending");
        let task_id = format!(
            "task-{}",
            json["submission_id"]
                .as_str()
                .expect("submission id should be a string")
        );
        let snapshot = runtime_service
            .get_task(&task_id)
            .expect("submission route should schedule a runtime task");
        assert_eq!(snapshot.queue, "oj_judge");
        assert_eq!(snapshot.lane, "fast");
        assert_eq!(snapshot.status, RuntimeTaskLifecycleStatus::Queued);
    }

    #[tokio::test]
    async fn update_problem_route_rejects_path_body_problem_id_mismatch() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service, None);

        let body = serde_json::json!({
            "problem_id": "body-problem",
            "title": "sum",
            "slug": "sum-http",
            "judge_mode": "acm",
            "statement_md": "demo",
            "supported_languages": ["cpp"],
            "limits": {
                "cpp": {
                    "time_limit_ms": 1000,
                    "memory_limit_kb": 262144
                }
            },
            "testcases": [{
                "case_no": 1,
                "input": "1 2\n",
                "expected_output": "3\n",
                "is_sample": true,
                "score": 100
            }],
            "judge_config": null,
            "easy_config": null
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/oj/problems/path-problem")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_problems_route_returns_problem_summaries() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service, None);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/oj/problems")
                    .body(Body::empty())
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert!(json.as_array().is_some());
        assert_eq!(json[0]["problem_id"], "two-sum");
        assert_eq!(json[0]["judge_mode"], "acm");
    }

    #[tokio::test]
    async fn get_problem_route_returns_problem_detail_shape() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service, None);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/oj/problems/two-sum")
                    .body(Body::empty())
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(json["problem"]["problem_id"], "two-sum");
        assert_eq!(json["problem"]["title"], "两数之和");
        assert_eq!(json["problem"]["supported_languages"][0], "cpp");
    }

    #[tokio::test]
    async fn catalog_routes_return_languages_and_templates() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        let router = build_router(build_default_catalog(), service, None);

        let languages_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/oj/catalog/languages")
                    .body(Body::empty())
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");
        assert_eq!(languages_response.status(), StatusCode::OK);
        let languages_body = to_bytes(languages_response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let languages_json: Value =
            serde_json::from_slice(&languages_body).expect("body should be valid json");
        assert!(languages_json.as_array().is_some());
        assert_eq!(languages_json[0]["key"], "cpp");

        let template_response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/oj/catalog/templates/cpp/acm")
                    .body(Body::empty())
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");
        assert_eq!(template_response.status(), StatusCode::OK);
        let template_body = to_bytes(template_response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let template_json: Value =
            serde_json::from_slice(&template_body).expect("body should be valid json");
        assert_eq!(template_json["language"], "cpp");
        assert_eq!(template_json["judge_mode"], "acm");
        assert!(template_json["template"]
            .as_str()
            .is_some_and(|value| value.contains("int main")));
    }

    #[tokio::test]
    async fn easy_judge_submission_route_returns_direct_result() {
        let service = Arc::new(OjService::new(
            Arc::new(InMemoryProblemRepository::seeded()),
            Arc::new(InMemorySubmissionRepository::default()),
        ));
        service
            .save_problem(crate::domain::Problem {
                problem_id: ProblemId::from("easy-1"),
                title: "easy".to_owned(),
                slug: "easy-1".to_owned(),
                judge_mode: crate::domain::JudgeMode::EasyJudge,
                sandbox_kind: crate::domain::SandboxKind::Nsjail,
                statement_md: "demo".to_owned(),
                supported_languages: vec![],
                limits: std::collections::BTreeMap::new(),
                testcases: vec![],
                judge_config: None,
                easy_config: Some(crate::domain::EasyProblemConfig {
                    question_type: crate::domain::EasyQuestionType::SingleChoice,
                    options: vec![
                        crate::domain::EasyOption {
                            key: "A".to_owned(),
                            label: "A".to_owned(),
                        },
                        crate::domain::EasyOption {
                            key: "B".to_owned(),
                            label: "B".to_owned(),
                        },
                    ],
                    standard_answer: crate::domain::EasyAnswer::Text("A".to_owned()),
                }),
            })
            .await
            .expect("problem should be saved");
        let router = build_router(build_default_catalog(), service, None);

        let body = serde_json::json!({
            "problem_id": "easy-1",
            "user_id": "u-easy",
            "answer": "A"
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/oj/easy-judge/submissions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request should be built"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(json["problem_id"], "easy-1");
        assert_eq!(json["status"], "accepted");
        assert_eq!(json["score"], 100);
    }

    fn runtime_event(
        submission_id: &str,
        status: RuntimeTaskLifecycleStatus,
        outcome: Option<RuntimeExecutionOutcome>,
        message: &str,
    ) -> RuntimeTaskEvent {
        RuntimeTaskEvent {
            task_id: "task-http-1".to_owned(),
            source_domain: "oj".to_owned(),
            queue: "oj_judge".to_owned(),
            lane: "fast".to_owned(),
            attempt: 1,
            submission_id: Some(submission_id.to_owned()),
            problem_id: Some("two-sum".to_owned()),
            user_id: Some("u-http".to_owned()),
            language: Some("cpp".to_owned()),
            status,
            message: message.to_owned(),
            execution_id: Some("rt-http-1".to_owned()),
            outcome,
        }
    }
}
