use serde::{Deserialize, Serialize};

use crate::model::status::JobStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobFailureDisposition {
    Retrying,
    DeadLettered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobFailure {
    pub reason: String,
    pub disposition: JobFailureDisposition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub status: JobStatus,
    pub failure: Option<JobFailure>,
}
