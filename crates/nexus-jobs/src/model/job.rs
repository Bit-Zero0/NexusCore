use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::payload::JobPayload;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct JobId(pub String);

impl From<String> for JobId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for JobId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobNamespace(pub String);

impl JobNamespace {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobType {
    pub namespace: String,
    pub name: String,
    pub version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobOrigin {
    pub source_domain: String,
    pub source_entity_id: String,
    pub submitted_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRoute {
    pub queue: String,
    pub lane: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRetryPolicy {
    pub max_attempts: u32,
    pub retry_delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JobTimeoutPolicy {
    pub execution_timeout_ms: Option<u64>,
    pub lease_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDispatch {
    pub route: JobRoute,
    pub retry_policy: JobRetryPolicy,
    pub timeout_policy: JobTimeoutPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDefinition {
    pub job_id: JobId,
    pub job_type: JobType,
    pub namespace: JobNamespace,
    pub origin: JobOrigin,
    pub dispatch: JobDispatch,
    pub payload: JobPayload,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobReceipt {
    pub job_id: String,
    pub queue: String,
    pub lane: String,
    pub handler: Option<String>,
    pub execution_contract: Option<String>,
}
