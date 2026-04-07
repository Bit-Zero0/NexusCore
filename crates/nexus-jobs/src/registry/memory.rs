use std::{collections::HashMap, sync::Mutex};

use crate::{model::JobDefinition, registry::JobDefinitionStore};

#[derive(Default)]
pub struct InMemoryJobDefinitionStore {
    jobs: Mutex<HashMap<String, JobDefinition>>,
}

impl JobDefinitionStore for InMemoryJobDefinitionStore {
    fn save(&self, job: JobDefinition) {
        if let Ok(mut guard) = self.jobs.lock() {
            guard.insert(job.job_id.0.clone(), job);
        }
    }

    fn get(&self, job_id: &str) -> Option<JobDefinition> {
        self.jobs
            .lock()
            .ok()
            .and_then(|guard| guard.get(job_id).cloned())
    }

    fn list(&self) -> Vec<JobDefinition> {
        self.jobs
            .lock()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default()
    }
}
