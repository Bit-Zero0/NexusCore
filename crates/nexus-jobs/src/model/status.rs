use serde::{Deserialize, Serialize};

use nexus_runtime::RuntimeTaskLifecycleStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Created,
    Enqueued,
    Leased,
    Running,
    Succeeded,
    Retrying,
    DeadLettered,
    Failed,
    Cancelled,
}

impl From<RuntimeTaskLifecycleStatus> for JobStatus {
    fn from(value: RuntimeTaskLifecycleStatus) -> Self {
        match value {
            RuntimeTaskLifecycleStatus::Queued => Self::Enqueued,
            RuntimeTaskLifecycleStatus::Retrying => Self::Retrying,
            RuntimeTaskLifecycleStatus::Preparing
            | RuntimeTaskLifecycleStatus::Prepared
            | RuntimeTaskLifecycleStatus::Compiling
            | RuntimeTaskLifecycleStatus::Running => Self::Running,
            RuntimeTaskLifecycleStatus::Completed => Self::Succeeded,
            RuntimeTaskLifecycleStatus::Failed => Self::Failed,
            RuntimeTaskLifecycleStatus::DeadLettered => Self::DeadLettered,
        }
    }
}
