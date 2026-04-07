use async_trait::async_trait;

use nexus_shared::AppResult;

use crate::model::{JobDefinition, JobReceipt};

#[async_trait]
pub trait JobSubmitter: Send + Sync {
    async fn submit(&self, job: JobDefinition) -> AppResult<JobReceipt>;
}

pub trait JobSubmissionValidator: Send + Sync {
    fn validate(&self, job: &JobDefinition) -> AppResult<()>;
}
