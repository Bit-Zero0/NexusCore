use std::{collections::HashMap, sync::Mutex};

use crate::{model::JobEvent, registry::JobEventStore};

#[derive(Default)]
pub struct InMemoryJobEventStore {
    events: Mutex<HashMap<String, Vec<JobEvent>>>,
}

impl JobEventStore for InMemoryJobEventStore {
    fn append(&self, event: JobEvent) {
        if let Ok(mut guard) = self.events.lock() {
            guard.entry(event.job_id.clone()).or_default().push(event);
        }
    }

    fn list(&self, job_id: &str) -> Vec<JobEvent> {
        self.events
            .lock()
            .ok()
            .and_then(|guard| guard.get(job_id).cloned())
            .unwrap_or_default()
    }
}
