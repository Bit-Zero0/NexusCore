use serde::{Deserialize, Serialize};

use crate::domains::OjJudgeJobPayload;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobJsonPayload {
    pub schema: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobPayload {
    OjJudge(OjJudgeJobPayload),
    Json(JobJsonPayload),
}
