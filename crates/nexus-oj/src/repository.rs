use async_trait::async_trait;
use nexus_runtime::RuntimeTaskEvent;

use crate::{
    domain::SubmissionStatus, EasySubmissionDraft, Problem, ProblemDetail, ProblemSummary,
    SubmissionDetail, SubmissionDraft, SubmissionRecord,
};
use nexus_shared::AppResult;

#[async_trait]
pub trait ProblemRepository: Send + Sync {
    async fn list_summaries(&self) -> AppResult<Vec<ProblemSummary>>;
    async fn find_detail(&self, problem_id: &str) -> AppResult<Option<ProblemDetail>>;
    async fn save(&self, problem: Problem) -> AppResult<()>;
}

#[async_trait]
pub trait SubmissionRepository: Send + Sync {
    async fn validate_submission(&self, draft: &SubmissionDraft) -> AppResult<()>;
    async fn list_submissions(&self) -> AppResult<Vec<SubmissionRecord>>;
    async fn find_submission(&self, submission_id: &str) -> AppResult<Option<SubmissionDetail>>;
    async fn create_submission(&self, draft: SubmissionDraft) -> AppResult<SubmissionRecord>;
    async fn apply_runtime_event(&self, event: &RuntimeTaskEvent) -> AppResult<()>;
    async fn create_direct_submission(
        &self,
        draft: EasySubmissionDraft,
        status: SubmissionStatus,
        score: u32,
        max_score: u32,
        message: Option<String>,
        stored_answer: String,
    ) -> AppResult<SubmissionRecord>;
}
