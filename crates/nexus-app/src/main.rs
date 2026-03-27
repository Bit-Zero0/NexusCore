mod telemetry;

use axum::{routing::get, Json, Router};
use nexus_config::{AppConfig, AppProcessRole};
use nexus_gateway::{
    build_gateway_services, build_router, build_router_with_services, map_runtime_worker_groups,
};
use nexus_shared::{AppResult, HealthStatus};
use nexus_storage::PostgresPoolFactory;
use redis::Client as RedisClient;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> AppResult<()> {
    telemetry::init();

    let config = AppConfig::from_env()?;
    if config.oj_repository == nexus_config::OjRepositoryMode::Postgres {
        let pool = PostgresPoolFactory::connect(&config.postgres).await?;
        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        PostgresPoolFactory::migrate(&pool, &migrations_dir).await?;
        info!(path = %migrations_dir.display(), "applied postgres migrations");
    }

    info!(
        env = %config.app_env,
        process_role = ?config.process_role,
        bind_addr = %config.server.bind_addr,
        "starting nexus-app"
    );

    match config.process_role {
        AppProcessRole::Embedded => {
            let listener = tokio::net::TcpListener::bind(config.server.bind_addr)
                .await
                .map_err(|_| nexus_shared::AppError::Internal)?;
            let (oj_service, runtime_service) = build_gateway_services(&config).await?;
            runtime_service.register_node(config.runtime.node_id.clone());
            runtime_service
                .start_background_workers(map_runtime_worker_groups(&config.runtime.worker_groups));
            spawn_runtime_node_heartbeat(&config, runtime_service.clone());
            let router = build_router_with_services(
                oj_service,
                runtime_service,
                RedisClient::open(config.redis.url.as_str()).ok(),
                true,
                &config.server.cors_allowed_origins,
            );
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .map_err(|_| nexus_shared::AppError::Internal)?;
        }
        AppProcessRole::Gateway => {
            let listener = tokio::net::TcpListener::bind(config.server.bind_addr)
                .await
                .map_err(|_| nexus_shared::AppError::Internal)?;
            let router = build_router(&config).await?;
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .map_err(|_| nexus_shared::AppError::Internal)?;
        }
        AppProcessRole::RuntimeWorker => {
            let listener = tokio::net::TcpListener::bind(config.server.bind_addr)
                .await
                .map_err(|_| nexus_shared::AppError::Internal)?;
            let (.., runtime_service) = build_gateway_services(&config).await?;
            runtime_service.register_node(config.runtime.node_id.clone());
            runtime_service
                .start_background_workers(map_runtime_worker_groups(&config.runtime.worker_groups));
            spawn_runtime_node_heartbeat(&config, runtime_service.clone());
            let router = Router::new()
                .route("/healthz", get(runtime_worker_healthz))
                .route("/api/v1/system/health", get(runtime_worker_healthz))
                .merge(nexus_runtime::build_router(runtime_service))
                .layer(TraceLayer::new_for_http());
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .map_err(|_| nexus_shared::AppError::Internal)?;
        }
    }

    Ok(())
}

async fn runtime_worker_healthz() -> Json<HealthStatus> {
    Json(HealthStatus::ok(
        "nexus-runtime-worker",
        env!("CARGO_PKG_VERSION"),
    ))
}

fn spawn_runtime_node_heartbeat(
    config: &AppConfig,
    runtime_service: Arc<nexus_runtime::RuntimeTaskService>,
) {
    let Ok(redis_client) = RedisClient::open(config.redis.url.as_str()) else {
        warn!("failed to create redis client for runtime node heartbeat");
        return;
    };

    tokio::spawn(async move {
        loop {
            let node_status = runtime_service.node_status();
            let key = format!("runtime_nodes:{}", node_status.node_id);
            let payload = match serde_json::to_string(&node_status) {
                Ok(payload) => payload,
                Err(error) => {
                    warn!(error = %error, "failed to serialize runtime node heartbeat");
                    sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

            match redis_client.get_multiplexed_async_connection().await {
                Ok(mut connection) => {
                    if let Err(error) = redis::pipe()
                        .cmd("SET")
                        .arg(&key)
                        .arg(&payload)
                        .cmd("EXPIRE")
                        .arg(&key)
                        .arg(30)
                        .cmd("PUBLISH")
                        .arg("runtime_node_heartbeats")
                        .arg(&payload)
                        .query_async::<()>(&mut connection)
                        .await
                    {
                        warn!(error = %error, key = %key, "failed to publish runtime node heartbeat");
                    }
                }
                Err(error) => {
                    warn!(error = %error, "failed to open redis connection for runtime heartbeat");
                }
            }

            sleep(Duration::from_secs(10)).await;
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};

        if let Ok(mut signal) = signal(SignalKind::terminate()) {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
