use serde::{Deserialize, Serialize};

use crate::{
    handlers::JobHandlerDescriptor,
    model::{JobDispatch, JobEvent, JobOrigin, JobStatus, JobType},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSubmissionFailureSnapshot {
    pub message: String,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub job_id: String,
    pub job_type: JobType,
    pub origin: JobOrigin,
    pub dispatch: JobDispatch,
    pub status: JobStatus,
    pub message: String,
    pub error: Option<String>,
    pub handler: Option<JobHandlerDescriptor>,
    pub latest_submission_failure: Option<JobSubmissionFailureSnapshot>,
    pub history: Vec<JobEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobManagementSummary {
    pub total_jobs: usize,
    pub queued: usize,
    pub running: usize,
    pub succeeded: usize,
    pub retrying: usize,
    pub dead_lettered: usize,
    pub failed: usize,
    pub submission_rejected: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobManagementView {
    pub jobs: Vec<JobSnapshot>,
    pub summary: JobManagementSummary,
}
