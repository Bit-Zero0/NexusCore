use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::handlers::{JobHandler, JobHandlerDescriptor};

pub trait JobHandlerRegistry: Send + Sync {
    fn register_descriptor(&self, descriptor: JobHandlerDescriptor);
    fn register_handler(&self, handler: Arc<dyn JobHandler>);
    fn resolve_descriptor(
        &self,
        namespace: &str,
        name: &str,
        version: u16,
    ) -> Option<JobHandlerDescriptor>;
    fn resolve_handler(
        &self,
        namespace: &str,
        name: &str,
        version: u16,
    ) -> Option<Arc<dyn JobHandler>>;
    fn list(&self) -> Vec<JobHandlerDescriptor>;
}

pub type SharedJobHandlerRegistry = Arc<dyn JobHandlerRegistry>;

#[derive(Default)]
pub struct InMemoryJobHandlerRegistry {
    descriptors: Mutex<HashMap<String, JobHandlerDescriptor>>,
    handlers: Mutex<HashMap<String, Arc<dyn JobHandler>>>,
}

impl JobHandlerRegistry for InMemoryJobHandlerRegistry {
    fn register_descriptor(&self, descriptor: JobHandlerDescriptor) {
        if let Ok(mut guard) = self.descriptors.lock() {
            guard.insert(
                handler_key(
                    descriptor.job_type.namespace.as_str(),
                    descriptor.job_type.name.as_str(),
                    descriptor.job_type.version,
                ),
                descriptor,
            );
        }
    }

    fn register_handler(&self, handler: Arc<dyn JobHandler>) {
        let descriptor = handler.descriptor();
        let key = handler_key(
            descriptor.job_type.namespace.as_str(),
            descriptor.job_type.name.as_str(),
            descriptor.job_type.version,
        );
        self.register_descriptor(descriptor);
        if let Ok(mut guard) = self.handlers.lock() {
            guard.insert(key, handler);
        }
    }

    fn resolve_descriptor(
        &self,
        namespace: &str,
        name: &str,
        version: u16,
    ) -> Option<JobHandlerDescriptor> {
        self.descriptors
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handler_key(namespace, name, version)).cloned())
    }

    fn resolve_handler(
        &self,
        namespace: &str,
        name: &str,
        version: u16,
    ) -> Option<Arc<dyn JobHandler>> {
        self.handlers
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handler_key(namespace, name, version)).cloned())
    }

    fn list(&self) -> Vec<JobHandlerDescriptor> {
        self.descriptors
            .lock()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default()
    }
}

fn handler_key(namespace: &str, name: &str, version: u16) -> String {
    format!("{namespace}:{name}:v{version}")
}
