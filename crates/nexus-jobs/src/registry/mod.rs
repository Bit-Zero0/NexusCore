mod events;
mod memory;

use std::sync::Arc;

use crate::model::{JobDefinition, JobEvent};

pub use events::InMemoryJobEventStore;
pub use memory::InMemoryJobDefinitionStore;

pub trait JobDefinitionStore: Send + Sync {
    fn save(&self, job: JobDefinition);
    fn get(&self, job_id: &str) -> Option<JobDefinition>;
    fn list(&self) -> Vec<JobDefinition>;
}

pub type SharedJobDefinitionStore = Arc<dyn JobDefinitionStore>;

pub trait JobEventStore: Send + Sync {
    fn append(&self, event: JobEvent);
    fn list(&self, job_id: &str) -> Vec<JobEvent>;
}

pub type SharedJobEventStore = Arc<dyn JobEventStore>;
