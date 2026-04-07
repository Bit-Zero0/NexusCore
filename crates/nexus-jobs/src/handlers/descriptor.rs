use serde::{Deserialize, Serialize};

use crate::model::JobType;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobExecutionContract {
    RuntimeTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobHandlerCapabilities {
    pub validates_payload: bool,
    pub requires_runtime_worker: bool,
    pub supports_dry_run: bool,
    pub emits_result_payload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobHandlerDescriptor {
    pub job_type: JobType,
    pub description: String,
    pub execution_contract: JobExecutionContract,
    pub supports_replay: bool,
    pub idempotent_submission: bool,
    pub capabilities: JobHandlerCapabilities,
}

impl JobHandlerDescriptor {
    pub fn key(&self) -> String {
        format!(
            "{}:{}:v{}",
            self.job_type.namespace, self.job_type.name, self.job_type.version
        )
    }
}
