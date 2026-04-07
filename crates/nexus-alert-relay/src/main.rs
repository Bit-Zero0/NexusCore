use std::{
    env,
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Clone)]
struct AppState {
    client: Client,
    generic_webhook_url: Option<String>,
    feishu_webhook_url: Option<String>,
    wecom_webhook_url: Option<String>,
    dry_run_enabled: bool,
    audit_log_path: Option<PathBuf>,
    audit_log_max_bytes: u64,
    audit_log_max_files: usize,
    audit_log_retention_days: u64,
    audit_log_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AlertmanagerWebhookPayload {
    #[serde(default)]
    status: String,
    #[serde(default)]
    alerts: Vec<AlertPayload>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AlertPayload {
    #[serde(default)]
    labels: Value,
    #[serde(default)]
    annotations: Value,
}

#[derive(Debug, Serialize)]
struct AuditRecord<'a> {
    ts_ms: u128,
    channel: &'a str,
    status: &'a str,
    alert_count: usize,
    text: String,
    markdown: String,
    payload: &'a AlertmanagerWebhookPayload,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct AlertRelayStatusResponse {
    status: &'static str,
    dry_run_enabled: bool,
    generic_webhook_configured: bool,
    feishu_webhook_configured: bool,
    wecom_webhook_configured: bool,
    audit_log_path: Option<String>,
    audit_log_max_bytes: u64,
    audit_log_max_files: usize,
    audit_log_retention_days: u64,
}

#[derive(Clone, Copy)]
enum Channel {
    DryRun,
    Generic,
    Feishu,
    Wecom,
}

#[tokio::main]
async fn main() {
    init_tracing();

    let bind_addr = env::var("NEXUS_ALERT_RELAY_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:18081".to_owned())
        .parse::<SocketAddr>()
        .expect("NEXUS_ALERT_RELAY_BIND_ADDR must be a valid socket address");

    let state = Arc::new(AppState {
        client: Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("http client must build"),
        generic_webhook_url: env_var("NEXUS_ALERT_WEBHOOK_URL"),
        feishu_webhook_url: env_var("NEXUS_ALERT_FEISHU_WEBHOOK_URL"),
        wecom_webhook_url: env_var("NEXUS_ALERT_WECOM_WEBHOOK_URL"),
        dry_run_enabled: env_bool("NEXUS_ALERT_DRY_RUN", true),
        audit_log_path: env_var("NEXUS_ALERT_AUDIT_LOG_PATH").map(PathBuf::from),
        audit_log_max_bytes: env_u64("NEXUS_ALERT_AUDIT_LOG_MAX_BYTES", 10 * 1024 * 1024),
        audit_log_max_files: env_usize("NEXUS_ALERT_AUDIT_LOG_MAX_FILES", 7),
        audit_log_retention_days: env_u64("NEXUS_ALERT_AUDIT_LOG_RETENTION_DAYS", 7),
        audit_log_lock: Arc::new(Mutex::new(())),
    });

    let app = Router::new()
        .route("/", get(healthz))
        .route("/healthz", get(healthz))
        .route("/status", get(status))
        .route("/alertmanager", post(fanout_alertmanager))
        .route("/dry-run", post(dry_run_webhook))
        .route("/generic", post(generic_webhook))
        .route("/feishu", post(feishu_webhook))
        .route("/wecom", post(wecom_webhook))
        .with_state(state)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(false)),
        );

    info!(bind_addr = %bind_addr, "starting nexus-alert-relay");

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("bind must succeed");
    axum::serve(listener, app).await.expect("server must run");
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).with_target(false).init();
}

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn status(State(state): State<Arc<AppState>>) -> Json<AlertRelayStatusResponse> {
    Json(AlertRelayStatusResponse {
        status: "ok",
        dry_run_enabled: state.dry_run_enabled,
        generic_webhook_configured: state.generic_webhook_url.is_some(),
        feishu_webhook_configured: state.feishu_webhook_url.is_some(),
        wecom_webhook_configured: state.wecom_webhook_url.is_some(),
        audit_log_path: state
            .audit_log_path
            .as_ref()
            .map(|path| path.display().to_string()),
        audit_log_max_bytes: state.audit_log_max_bytes,
        audit_log_max_files: state.audit_log_max_files,
        audit_log_retention_days: state.audit_log_retention_days,
    })
}

async fn fanout_alertmanager(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AlertmanagerWebhookPayload>,
) -> impl IntoResponse {
    if let Err(error) = forward(&state, Channel::Generic, &payload).await {
        error!(error = %error, "failed to forward alert payload to generic webhook");
        return (StatusCode::BAD_GATEWAY, Json(json!({ "forwarded": false })));
    }
    if let Err(error) = forward(&state, Channel::Feishu, &payload).await {
        error!(error = %error, "failed to forward alert payload to feishu webhook");
        return (StatusCode::BAD_GATEWAY, Json(json!({ "forwarded": false })));
    }
    if let Err(error) = forward(&state, Channel::Wecom, &payload).await {
        error!(error = %error, "failed to forward alert payload to wecom webhook");
        return (StatusCode::BAD_GATEWAY, Json(json!({ "forwarded": false })));
    }
    (StatusCode::OK, Json(json!({ "forwarded": true })))
}

async fn generic_webhook(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AlertmanagerWebhookPayload>,
) -> impl IntoResponse {
    respond_forward(forward(&state, Channel::Generic, &payload).await)
}

async fn dry_run_webhook(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AlertmanagerWebhookPayload>,
) -> impl IntoResponse {
    respond_forward(forward(&state, Channel::DryRun, &payload).await)
}

async fn feishu_webhook(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AlertmanagerWebhookPayload>,
) -> impl IntoResponse {
    respond_forward(forward(&state, Channel::Feishu, &payload).await)
}

async fn wecom_webhook(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AlertmanagerWebhookPayload>,
) -> impl IntoResponse {
    respond_forward(forward(&state, Channel::Wecom, &payload).await)
}

fn respond_forward(result: Result<(), reqwest::Error>) -> impl IntoResponse {
    match result {
        Ok(()) => (StatusCode::OK, Json(json!({ "forwarded": true }))),
        Err(error) => {
            error!(error = %error, "failed to forward alert payload");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "forwarded": false, "error": error.to_string() })),
            )
        }
    }
}

async fn forward(
    state: &AppState,
    channel: Channel,
    payload: &AlertmanagerWebhookPayload,
) -> Result<(), reqwest::Error> {
    if matches!(channel, Channel::DryRun) {
        write_audit_log(state, channel, payload).await;
        log_dry_run(payload);
        return Ok(());
    }

    let Some(url) = webhook_url(state, channel) else {
        if state.dry_run_enabled {
            write_audit_log(state, channel, payload).await;
            log_dry_run(payload);
        }
        return Ok(());
    };
    let body = webhook_body(channel, payload);
    let response = state.client.post(url).json(&body).send().await?;
    write_audit_log(state, channel, payload).await;
    if response.status().is_success() {
        info!(channel = channel_name(channel), "forwarded alert payload");
    } else {
        warn!(
            channel = channel_name(channel),
            status = %response.status(),
            "alert webhook responded with non-success status"
        );
    }
    Ok(())
}

fn webhook_url(state: &AppState, channel: Channel) -> Option<&str> {
    match channel {
        Channel::DryRun => None,
        Channel::Generic => state.generic_webhook_url.as_deref(),
        Channel::Feishu => state.feishu_webhook_url.as_deref(),
        Channel::Wecom => state.wecom_webhook_url.as_deref(),
    }
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::DryRun => "dry-run",
        Channel::Generic => "generic",
        Channel::Feishu => "feishu",
        Channel::Wecom => "wecom",
    }
}

fn webhook_body(channel: Channel, payload: &AlertmanagerWebhookPayload) -> Value {
    match channel {
        Channel::DryRun => json!(payload),
        Channel::Generic => json!(payload),
        Channel::Feishu => json!({
            "msg_type": "text",
            "content": {
                "text": alert_text(payload),
            }
        }),
        Channel::Wecom => json!({
            "msgtype": "markdown",
            "markdown": {
                "content": alert_markdown(payload),
            }
        }),
    }
}

async fn write_audit_log(state: &AppState, channel: Channel, payload: &AlertmanagerWebhookPayload) {
    let Some(path) = state.audit_log_path.as_ref() else {
        return;
    };

    let record = AuditRecord {
        ts_ms: now_ms(),
        channel: channel_name(channel),
        status: payload.status.as_str(),
        alert_count: payload.alerts.len(),
        text: alert_text(payload),
        markdown: alert_markdown(payload),
        payload,
    };

    let Ok(line) = serde_json::to_string(&record) else {
        warn!(
            channel = channel_name(channel),
            "failed to serialize alert audit record"
        );
        return;
    };

    let _guard = state.audit_log_lock.lock().await;
    let Some(parent) = path.parent() else {
        warn!(path = %path.display(), "alert audit log path has no parent directory");
        return;
    };
    if let Err(error) = tokio::fs::create_dir_all(parent).await {
        warn!(error = %error, path = %parent.display(), "failed to create alert audit log directory");
        return;
    }

    let target_path = rotated_audit_log_path(path, state.audit_log_max_bytes).await;

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&target_path)
        .await
    {
        Ok(mut file) => {
            let line = format!("{line}\n");
            if let Err(error) = file.write_all(line.as_bytes()).await {
                warn!(error = %error, path = %target_path.display(), "failed to write alert audit log");
            }
        }
        Err(error) => {
            warn!(error = %error, path = %target_path.display(), "failed to open alert audit log");
        }
    }

    prune_rotated_audit_logs(
        path.as_path(),
        state.audit_log_max_files,
        state.audit_log_retention_days,
    )
    .await;
}

async fn rotated_audit_log_path(base_path: &PathBuf, max_bytes: u64) -> PathBuf {
    let metadata = match tokio::fs::metadata(base_path).await {
        Ok(metadata) => metadata,
        Err(_) => return base_path.clone(),
    };

    if metadata.len() < max_bytes {
        return base_path.clone();
    }

    let parent = base_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = base_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("audit");
    let extension = base_path.extension().and_then(|value| value.to_str());
    let suffix = now_ms();

    let file_name = match extension {
        Some(extension) => format!("{stem}-{suffix}.{extension}"),
        None => format!("{stem}-{suffix}"),
    };

    parent.join(file_name)
}

async fn prune_rotated_audit_logs(base_path: &Path, max_files: usize, retention_days: u64) {
    if max_files == 0 && retention_days == 0 {
        return;
    }

    let Some(parent) = base_path.parent() else {
        return;
    };
    let Some(stem) = base_path.file_stem().and_then(|value| value.to_str()) else {
        return;
    };
    let extension = base_path.extension().and_then(|value| value.to_str());

    let mut read_dir = match tokio::fs::read_dir(parent).await {
        Ok(read_dir) => read_dir,
        Err(error) => {
            warn!(error = %error, path = %parent.display(), "failed to scan audit log directory");
            return;
        }
    };

    let mut rotated_files: Vec<(PathBuf, OsString, Option<SystemTime>)> = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let path = entry.path();
        if !matches_rotated_audit_file(&path, stem, extension) {
            continue;
        }
        let modified_at = entry
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        let file_name = entry.file_name();
        rotated_files.push((path, file_name, modified_at));
    }

    if retention_days > 0 {
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(retention_days * 24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
        for (path, _, modified_at) in &rotated_files {
            if modified_at.is_some_and(|value| value < cutoff) {
                if let Err(error) = tokio::fs::remove_file(path).await {
                    warn!(error = %error, path = %path.display(), "failed to prune expired rotated audit log");
                }
            }
        }
        rotated_files.retain(|(path, _, _)| path.exists());
    }

    if max_files == 0 || rotated_files.len() <= max_files {
        return;
    }

    rotated_files.sort_by(|left, right| left.1.cmp(&right.1));
    let to_remove = rotated_files.len().saturating_sub(max_files);
    for (path, _, _) in rotated_files.into_iter().take(to_remove) {
        if let Err(error) = tokio::fs::remove_file(&path).await {
            warn!(error = %error, path = %path.display(), "failed to prune rotated audit log");
        }
    }
}

fn matches_rotated_audit_file(path: &Path, stem: &str, extension: Option<&str>) -> bool {
    let Some(file_stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    if !file_stem.starts_with(&format!("{stem}-")) {
        return false;
    }

    match (extension, path.extension().and_then(|value| value.to_str())) {
        (Some(expected), Some(actual)) => actual == expected,
        (None, None) => true,
        _ => false,
    }
}

fn log_dry_run(payload: &AlertmanagerWebhookPayload) {
    info!(
        status = %payload.status,
        alert_count = payload.alerts.len(),
        text = %alert_text(payload),
        markdown = %alert_markdown(payload),
        "dry-run alert receiver captured payload"
    );
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn alert_text(payload: &AlertmanagerWebhookPayload) -> String {
    let mut lines = vec![format!(
        "[Nexus Alert] status={} count={}",
        payload.status,
        payload.alerts.len()
    )];
    for alert in &payload.alerts {
        let name = label(&alert.labels, "alertname");
        let severity = label(&alert.labels, "severity");
        let broker = label(&alert.labels, "broker");
        let queue = label(&alert.labels, "queue");
        let lane = label(&alert.labels, "lane");
        let summary = label(&alert.annotations, "summary");
        lines.push(format!(
            "- {} severity={} broker={} queue={} lane={} {}",
            name, severity, broker, queue, lane, summary
        ));
    }
    lines.join("\n")
}

fn alert_markdown(payload: &AlertmanagerWebhookPayload) -> String {
    let mut lines = vec![
        "**Nexus Alert**".to_owned(),
        format!("> status: `{}`", payload.status),
        format!("> count: `{}`", payload.alerts.len()),
        String::new(),
    ];
    for alert in &payload.alerts {
        lines.push(format!("- **{}**", label(&alert.labels, "alertname")));
        lines.push(format!(
            "  - severity: `{}`",
            label(&alert.labels, "severity")
        ));
        lines.push(format!("  - broker: `{}`", label(&alert.labels, "broker")));
        lines.push(format!(
            "  - queue/lane: `{}` / `{}`",
            label(&alert.labels, "queue"),
            label(&alert.labels, "lane")
        ));
        lines.push(format!(
            "  - summary: {}",
            label(&alert.annotations, "summary")
        ));
    }
    lines.join("\n")
}

fn label(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_owned()
}
