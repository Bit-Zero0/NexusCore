use std::sync::Arc;

use nexus_shared::AppResult;

use crate::{
    api::{JobSubmissionValidator, JobSubmitter},
    model::{JobDefinition, JobEvent, JobEventMetadata, JobEventSource, JobReceipt, JobStatus},
    registry::{SharedJobDefinitionStore, SharedJobEventStore},
};

pub struct JobPlatformService {
    submitter: Arc<dyn JobSubmitter>,
    validator: Arc<dyn JobSubmissionValidator>,
    definition_store: SharedJobDefinitionStore,
    event_store: SharedJobEventStore,
}

impl JobPlatformService {
    pub fn new(
        submitter: Arc<dyn JobSubmitter>,
        validator: Arc<dyn JobSubmissionValidator>,
        definition_store: SharedJobDefinitionStore,
        event_store: SharedJobEventStore,
    ) -> Self {
        Self {
            submitter,
            validator,
            definition_store,
            event_store,
        }
    }

    pub async fn submit(&self, job: JobDefinition) -> AppResult<JobReceipt> {
        self.validator.validate(&job)?;
        self.definition_store.save(job.clone());
        let receipt = match self.submitter.submit(job.clone()).await {
            Ok(receipt) => receipt,
            Err(error) => {
                self.event_store.append(JobEvent {
                    job_id: job.job_id.0.clone(),
                    status: JobStatus::Failed,
                    source: JobEventSource::JobPlatform,
                    message: "job submission rejected before runtime dispatch".to_owned(),
                    attempt: Some(1),
                    occurred_at_ms: now_ms(),
                    error: Some(error.to_string()),
                    metadata: Some(JobEventMetadata {
                        queue: Some(job.dispatch.route.queue.clone()),
                        lane: Some(job.dispatch.route.lane.clone()),
                        source_domain: Some(job.origin.source_domain.clone()),
                        source_entity_id: Some(job.origin.source_entity_id.clone()),
                        handler: None,
                        execution_contract: None,
                    }),
                });
                return Err(error);
            }
        };
        self.event_store.append(JobEvent {
            job_id: receipt.job_id.clone(),
            status: JobStatus::Enqueued,
            source: JobEventSource::JobPlatform,
            message: build_dispatch_message(&receipt),
            attempt: Some(1),
            occurred_at_ms: now_ms(),
            error: None,
            metadata: Some(JobEventMetadata {
                queue: Some(receipt.queue.clone()),
                lane: Some(receipt.lane.clone()),
                source_domain: Some(job.origin.source_domain.clone()),
                source_entity_id: Some(job.origin.source_entity_id.clone()),
                handler: receipt.handler.clone(),
                execution_contract: receipt.execution_contract.clone(),
            }),
        });
        Ok(receipt)
    }
}

fn build_dispatch_message(receipt: &JobReceipt) -> String {
    match (&receipt.handler, &receipt.execution_contract) {
        (Some(handler), Some(contract)) => format!(
            "job accepted by job platform and dispatched to runtime via {handler} ({contract})"
        ),
        (Some(handler), None) => {
            format!("job accepted by job platform and dispatched to runtime via {handler}")
        }
        _ => "job accepted by job platform and dispatched to runtime".to_owned(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
