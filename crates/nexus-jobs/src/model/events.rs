use serde::{Deserialize, Serialize};

use crate::model::JobStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEventSource {
    JobPlatform,
    Runtime,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobEventMetadata {
    pub queue: Option<String>,
    pub lane: Option<String>,
    pub source_domain: Option<String>,
    pub source_entity_id: Option<String>,
    pub handler: Option<String>,
    pub execution_contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEvent {
    pub job_id: String,
    pub status: JobStatus,
    pub source: JobEventSource,
    pub message: String,
    pub attempt: Option<u32>,
    pub occurred_at_ms: u64,
    pub error: Option<String>,
    pub metadata: Option<JobEventMetadata>,
}
