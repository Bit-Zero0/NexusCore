use std::env;

use tracing_subscriber::{fmt, EnvFilter};

pub fn init() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,nexus_runtime=info"));
    let log_format = env::var("NEXUS_LOG_FORMAT").unwrap_or_else(|_| "pretty".to_owned());

    let builder = fmt().with_env_filter(env_filter).with_target(false);
    let _ = match log_format.as_str() {
        "json" => builder.json().flatten_event(true).try_init(),
        _ => builder.compact().try_init(),
    };
}
