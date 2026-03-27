use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub service: &'static str,
    pub version: &'static str,
    pub status: &'static str,
}

impl HealthStatus {
    pub fn ok(service: &'static str, version: &'static str) -> Self {
        Self {
            service,
            version,
            status: "ok",
        }
    }
}
