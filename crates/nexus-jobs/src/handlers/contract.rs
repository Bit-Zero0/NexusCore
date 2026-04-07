use std::collections::BTreeMap;

use async_trait::async_trait;
use nexus_runtime::RuntimeTask;
use nexus_shared::AppResult;

use crate::{
    handlers::{JobExecutionContract, JobHandlerDescriptor},
    model::{JobDefinition, JobOrigin, JobRetryPolicy, JobRoute, JobTimeoutPolicy},
};

#[derive(Debug, Clone)]
pub struct JobExecutionContext {
    pub job_id: String,
    pub namespace: String,
    pub route: JobRoute,
    pub retry_policy: JobRetryPolicy,
    pub timeout_policy: JobTimeoutPolicy,
    pub origin: JobOrigin,
    pub labels: BTreeMap<String, String>,
    pub submitted_at_ms: u64,
}

impl JobExecutionContext {
    pub fn from_job(job: &JobDefinition, submitted_at_ms: u64) -> Self {
        Self {
            job_id: job.job_id.0.clone(),
            namespace: job.namespace.0.clone(),
            route: job.dispatch.route.clone(),
            retry_policy: job.dispatch.retry_policy.clone(),
            timeout_policy: job.dispatch.timeout_policy.clone(),
            origin: job.origin.clone(),
            labels: job.labels.clone(),
            submitted_at_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobDispatchPlan {
    pub runtime_task: RuntimeTask,
    pub handler_descriptor: JobHandlerDescriptor,
    pub execution_contract: JobExecutionContract,
}

impl JobDispatchPlan {
    pub fn runtime_task(
        runtime_task: RuntimeTask,
        handler_descriptor: JobHandlerDescriptor,
    ) -> Self {
        Self {
            runtime_task,
            execution_contract: handler_descriptor.execution_contract.clone(),
            handler_descriptor,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobHandlerFailure {
    pub code: String,
    pub reason: String,
    pub retryable: bool,
}

impl JobHandlerFailure {
    pub fn rejected(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
            retryable: false,
        }
    }

    pub fn temporary(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
            retryable: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobHandlerResult {
    Dispatch(JobDispatchPlan),
    Rejected(JobHandlerFailure),
}

#[async_trait]
pub trait JobHandler: Send + Sync {
    fn descriptor(&self) -> JobHandlerDescriptor;

    async fn prepare_dispatch(
        &self,
        job: &JobDefinition,
        context: &JobExecutionContext,
    ) -> AppResult<JobHandlerResult>;
}
