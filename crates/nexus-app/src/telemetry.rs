use tracing_subscriber::{fmt, EnvFilter};

pub fn init() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));

    let _ = fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init();
}
