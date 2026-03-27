use std::sync::Arc;

use async_trait::async_trait;
use axum::{extract::Query, http::Method, routing::get, Json, Router};
use nexus_auth::build_dev_auth_service;
use nexus_config::{
    AppConfig, OjRepositoryMode, RuntimeQueueBackend as ConfigRuntimeQueueBackend,
    RuntimeSeccompMode as ConfigRuntimeSeccompMode,
    RuntimeSyscallFlavor as ConfigRuntimeSyscallFlavor, RuntimeWorkerGroupConfig,
};
use nexus_oj::{
    build_default_catalog, build_router as build_oj_router, InMemoryProblemRepository,
    InMemorySubmissionRepository, OjService, PgProblemRepository, PgSubmissionRepository,
};
use nexus_runtime::{
    build_default_runtime_catalog, build_rabbitmq_runtime_queue,
    build_router as build_runtime_router, build_runtime_queue, RabbitMqQueueConfig,
    RuntimeEventObserver, RuntimeNodeHealthStatus, RuntimeNodeStatus, RuntimeQueueBackend,
    RuntimeRouteBinding, RuntimeSeccompMode, RuntimeSyscallFlavor, RuntimeTaskEvent,
    RuntimeTaskService, RuntimeWorker, RuntimeWorkerGroup,
};
use nexus_shared::HealthStatus;
use nexus_storage::PostgresPoolFactory;
use redis::Client as RedisClient;
use serde::{Deserialize, Serialize};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{info, warn};

#[derive(Debug, Default, Deserialize)]
struct RuntimeNodeRegistryQuery {
    queue: Option<String>,
    lane: Option<String>,
    group: Option<String>,
    status: Option<RuntimeNodeHealthStatus>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RuntimeNodeRegistrySummary {
    total_nodes: usize,
    total_worker_groups: usize,
    healthy_nodes: usize,
    stale_nodes: usize,
    routes: Vec<RuntimeRouteCoverage>,
    groups: Vec<RuntimeGroupCoverage>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RuntimeRouteCoverage {
    queue: String,
    lane: String,
    node_count: usize,
    worker_group_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RuntimeGroupCoverage {
    name: String,
    node_count: usize,
    binding_count: usize,
}

const RUNTIME_NODE_STALE_AFTER_MS: u64 = 20_000;

pub async fn build_router(config: &AppConfig) -> nexus_shared::AppResult<Router> {
    let redis_client = RedisClient::open(config.redis.url.as_str()).ok();
    let (oj_service, runtime_service) = build_gateway_services(config).await?;
    Ok(build_router_with_services(
        oj_service,
        runtime_service,
        redis_client,
        false,
        &config.server.cors_allowed_origins,
    ))
}

pub fn build_router_with_services(
    oj_service: Arc<OjService>,
    runtime_service: Arc<RuntimeTaskService>,
    redis_client: Option<RedisClient>,
    expose_runtime_api: bool,
    cors_allowed_origins: &[String],
) -> Router {
    let runtime_nodes_route = {
        let redis_client = redis_client.clone();
        get(move |query| list_runtime_nodes(query, redis_client.clone()))
    };
    let runtime_nodes_summary_route = {
        let redis_client = redis_client.clone();
        get(move |query| list_runtime_node_summary(query, redis_client.clone()))
    };

    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/system/health", get(healthz))
        .route("/api/v1/runtime/nodes", runtime_nodes_route)
        .route("/api/v1/runtime/nodes/summary", runtime_nodes_summary_route)
        .merge(nexus_auth::build_router(build_dev_auth_service()))
        .merge(build_oj_router(
            build_default_catalog(),
            oj_service,
            Some(runtime_service.clone()),
        ))
        .layer(TraceLayer::new_for_http());

    let router = if expose_runtime_api {
        router.merge(build_runtime_router(runtime_service))
    } else {
        router
    };

    apply_cors_layer(router, cors_allowed_origins)
}

pub async fn build_gateway_services(
    config: &AppConfig,
) -> nexus_shared::AppResult<(Arc<OjService>, Arc<RuntimeTaskService>)> {
    let redis_client = RedisClient::open(config.redis.url.as_str()).ok();
    let (oj_service, runtime_observer) =
        build_oj_service_and_observer(config, redis_client).await?;
    let runtime_service = Arc::new(RuntimeTaskService::with_queue(
        Arc::new(RuntimeWorker::new(
            build_default_runtime_catalog(),
            config.runtime.work_root.clone(),
            config.runtime.nsjail_path.clone(),
            map_runtime_seccomp_mode(config.runtime.seccomp_mode),
            map_runtime_syscall_flavor(config.runtime.syscall_flavor),
        )),
        build_runtime_queue_from_config(config).await?,
        runtime_observer,
    ));

    Ok((oj_service, runtime_service))
}

fn map_runtime_seccomp_mode(mode: ConfigRuntimeSeccompMode) -> RuntimeSeccompMode {
    match mode {
        ConfigRuntimeSeccompMode::Log => RuntimeSeccompMode::Log,
        ConfigRuntimeSeccompMode::Kill => RuntimeSeccompMode::Kill,
    }
}

fn map_runtime_syscall_flavor(mode: ConfigRuntimeSyscallFlavor) -> RuntimeSyscallFlavor {
    match mode {
        ConfigRuntimeSyscallFlavor::Auto => RuntimeSyscallFlavor::Auto,
        ConfigRuntimeSyscallFlavor::Generic => RuntimeSyscallFlavor::Generic,
        ConfigRuntimeSyscallFlavor::DebianUbuntu => RuntimeSyscallFlavor::DebianUbuntu,
        ConfigRuntimeSyscallFlavor::Arch => RuntimeSyscallFlavor::Arch,
        ConfigRuntimeSyscallFlavor::RhelLike => RuntimeSyscallFlavor::RhelLike,
    }
}

async fn build_oj_service_and_observer(
    config: &AppConfig,
    redis_client: Option<RedisClient>,
) -> nexus_shared::AppResult<(Arc<OjService>, Arc<dyn RuntimeEventObserver>)> {
    Ok(match config.oj_repository {
        OjRepositoryMode::Memory => {
            info!("using in-memory OJ repositories");
            let oj_service = Arc::new(OjService::new(
                Arc::new(InMemoryProblemRepository::seeded()),
                Arc::new(InMemorySubmissionRepository::default()),
            ));
            (
                oj_service.clone(),
                Arc::new(GatewayRuntimeObserver::new(oj_service, redis_client))
                    as Arc<dyn RuntimeEventObserver>,
            )
        }
        OjRepositoryMode::Postgres => {
            let pool = PostgresPoolFactory::connect(&config.postgres).await?;
            PostgresPoolFactory::ping(&pool).await?;
            info!("using postgres OJ repositories");
            let oj_service = Arc::new(OjService::new(
                Arc::new(PgProblemRepository::new(pool.clone())),
                Arc::new(PgSubmissionRepository::new(pool.clone())),
            ));
            (
                oj_service.clone(),
                Arc::new(GatewayRuntimeObserver::new(oj_service, redis_client))
                    as Arc<dyn RuntimeEventObserver>,
            )
        }
    })
}

pub fn map_runtime_worker_groups(groups: &[RuntimeWorkerGroupConfig]) -> Vec<RuntimeWorkerGroup> {
    groups
        .iter()
        .map(|group| RuntimeWorkerGroup {
            name: group.name.clone(),
            bindings: group
                .bindings
                .iter()
                .map(|binding| RuntimeRouteBinding {
                    queue: binding.queue.clone(),
                    lane: binding.lane.clone(),
                })
                .collect(),
        })
        .collect()
}

async fn healthz() -> Json<HealthStatus> {
    Json(HealthStatus::ok("nexus-gateway", env!("CARGO_PKG_VERSION")))
}

async fn list_runtime_nodes(
    Query(query): Query<RuntimeNodeRegistryQuery>,
    redis_client: Option<RedisClient>,
) -> Json<Vec<RuntimeNodeStatus>> {
    let Some(redis_client) = redis_client else {
        return Json(Vec::new());
    };

    let mut nodes = match fetch_runtime_nodes(&redis_client).await {
        Ok(nodes) => nodes,
        Err(error) => {
            warn!(error = %error, "failed to fetch runtime nodes");
            Vec::new()
        }
    };
    nodes = annotate_runtime_node_health(nodes, current_time_ms());
    nodes = filter_runtime_nodes(nodes, &query);
    nodes.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    Json(nodes)
}

async fn list_runtime_node_summary(
    Query(query): Query<RuntimeNodeRegistryQuery>,
    redis_client: Option<RedisClient>,
) -> Json<RuntimeNodeRegistrySummary> {
    let Some(redis_client) = redis_client else {
        return Json(RuntimeNodeRegistrySummary {
            total_nodes: 0,
            total_worker_groups: 0,
            healthy_nodes: 0,
            stale_nodes: 0,
            routes: Vec::new(),
            groups: Vec::new(),
        });
    };

    let nodes = match fetch_runtime_nodes(&redis_client).await {
        Ok(nodes) => nodes,
        Err(error) => {
            warn!(error = %error, "failed to fetch runtime nodes for summary");
            Vec::new()
        }
    };

    let nodes = annotate_runtime_node_health(nodes, current_time_ms());
    Json(summarize_runtime_nodes(&filter_runtime_nodes(
        nodes, &query,
    )))
}

async fn fetch_runtime_nodes(
    client: &RedisClient,
) -> nexus_shared::AppResult<Vec<RuntimeNodeStatus>> {
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| nexus_shared::AppError::Internal)?;

    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("runtime_nodes:*")
        .query_async(&mut connection)
        .await
        .map_err(|_| nexus_shared::AppError::Internal)?;
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let values: Vec<String> = redis::cmd("MGET")
        .arg(&keys)
        .query_async(&mut connection)
        .await
        .map_err(|_| nexus_shared::AppError::Internal)?;

    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_str::<RuntimeNodeStatus>(&value).ok())
        .collect())
}

fn filter_runtime_nodes(
    nodes: Vec<RuntimeNodeStatus>,
    query: &RuntimeNodeRegistryQuery,
) -> Vec<RuntimeNodeStatus> {
    nodes
        .into_iter()
        .filter(|node| matches_node_query(node, query))
        .collect()
}

fn matches_node_query(node: &RuntimeNodeStatus, query: &RuntimeNodeRegistryQuery) -> bool {
    let status_matches = query
        .status
        .as_ref()
        .map_or(true, |status| status == &node.node_status);
    status_matches
        && node
            .worker_groups
            .iter()
            .any(|group| matches_group_query(group, query))
}

fn matches_group_query(group: &RuntimeWorkerGroup, query: &RuntimeNodeRegistryQuery) -> bool {
    let group_matches = query
        .group
        .as_deref()
        .map_or(true, |value| value == group.name);
    let route_matches = if query.queue.is_none() && query.lane.is_none() {
        true
    } else {
        group.bindings.iter().any(|binding| {
            query
                .queue
                .as_deref()
                .map_or(true, |value| value == binding.queue)
                && query
                    .lane
                    .as_deref()
                    .map_or(true, |value| value == binding.lane)
        })
    };
    group_matches && route_matches
}

fn summarize_runtime_nodes(nodes: &[RuntimeNodeStatus]) -> RuntimeNodeRegistrySummary {
    use std::collections::{BTreeMap, BTreeSet};

    let mut route_nodes: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut route_groups: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut group_nodes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut group_bindings: BTreeMap<String, usize> = BTreeMap::new();

    for node in nodes {
        for group in &node.worker_groups {
            group_nodes
                .entry(group.name.clone())
                .or_default()
                .insert(node.node_id.clone());
            group_bindings.insert(group.name.clone(), group.bindings.len());

            for binding in &group.bindings {
                let route = (binding.queue.clone(), binding.lane.clone());
                route_nodes
                    .entry(route.clone())
                    .or_default()
                    .insert(node.node_id.clone());
                *route_groups.entry(route).or_default() += 1;
            }
        }
    }

    let routes = route_nodes
        .into_iter()
        .map(|((queue, lane), node_ids)| RuntimeRouteCoverage {
            worker_group_count: route_groups
                .get(&(queue.clone(), lane.clone()))
                .copied()
                .unwrap_or_default(),
            node_count: node_ids.len(),
            queue,
            lane,
        })
        .collect();
    let groups = group_nodes
        .into_iter()
        .map(|(name, node_ids)| RuntimeGroupCoverage {
            binding_count: group_bindings.get(&name).copied().unwrap_or_default(),
            node_count: node_ids.len(),
            name,
        })
        .collect();

    RuntimeNodeRegistrySummary {
        total_nodes: nodes.len(),
        total_worker_groups: nodes.iter().map(|node| node.worker_groups.len()).sum(),
        healthy_nodes: nodes
            .iter()
            .filter(|node| node.node_status == RuntimeNodeHealthStatus::Healthy)
            .count(),
        stale_nodes: nodes
            .iter()
            .filter(|node| node.node_status == RuntimeNodeHealthStatus::Stale)
            .count(),
        routes,
        groups,
    }
}

fn annotate_runtime_node_health(
    nodes: Vec<RuntimeNodeStatus>,
    now_ms: u64,
) -> Vec<RuntimeNodeStatus> {
    nodes
        .into_iter()
        .map(|mut node| {
            let age_ms = now_ms.saturating_sub(node.last_heartbeat_ms);
            node.node_status = if age_ms > RUNTIME_NODE_STALE_AFTER_MS {
                RuntimeNodeHealthStatus::Stale
            } else {
                RuntimeNodeHealthStatus::Healthy
            };
            node
        })
        .collect()
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn apply_cors_layer(router: Router, allowed_origins: &[String]) -> Router {
    let Some(layer) = build_cors_layer(allowed_origins) else {
        return router;
    };
    router.layer(layer)
}

fn build_cors_layer(allowed_origins: &[String]) -> Option<CorsLayer> {
    if allowed_origins.is_empty() {
        return None;
    }

    let base = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS])
        .allow_headers(Any);

    if allowed_origins.iter().any(|origin| origin == "*") {
        return Some(base.allow_origin(Any));
    }

    let origins = allowed_origins
        .iter()
        .map(|origin| origin.parse())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;

    Some(base.allow_origin(origins))
}

fn map_runtime_queue_backend(backend: ConfigRuntimeQueueBackend) -> RuntimeQueueBackend {
    match backend {
        ConfigRuntimeQueueBackend::Memory => RuntimeQueueBackend::Memory,
        ConfigRuntimeQueueBackend::RabbitMq => RuntimeQueueBackend::Memory,
    }
}

async fn build_runtime_queue_from_config(
    config: &AppConfig,
) -> nexus_shared::AppResult<Arc<dyn nexus_runtime::RuntimeTaskQueue>> {
    match config.runtime.queue_backend {
        ConfigRuntimeQueueBackend::Memory => Ok(build_runtime_queue(map_runtime_queue_backend(
            config.runtime.queue_backend,
        ))),
        ConfigRuntimeQueueBackend::RabbitMq => {
            build_rabbitmq_runtime_queue(RabbitMqQueueConfig {
                url: config.runtime.rabbitmq.url.clone(),
                exchange: config.runtime.rabbitmq.exchange.clone(),
                queue_prefix: config.runtime.rabbitmq.queue_prefix.clone(),
            })
            .await
        }
    }
}

struct GatewayRuntimeObserver {
    oj_service: Arc<OjService>,
    redis_client: Option<RedisClient>,
}

impl GatewayRuntimeObserver {
    fn new(oj_service: Arc<OjService>, redis_client: Option<RedisClient>) -> Self {
        Self {
            oj_service,
            redis_client,
        }
    }
}

#[async_trait]
impl RuntimeEventObserver for GatewayRuntimeObserver {
    async fn on_event(&self, event: RuntimeTaskEvent) -> nexus_shared::AppResult<()> {
        self.oj_service.apply_runtime_event(&event).await?;

        if let (Some(submission_id), Some(redis_client)) =
            (&event.submission_id, &self.redis_client)
        {
            publish_runtime_event(redis_client, submission_id, &event).await;
        }

        Ok(())
    }
}

async fn publish_runtime_event(
    client: &RedisClient,
    submission_id: &str,
    event: &RuntimeTaskEvent,
) {
    let channel = format!("submission_updates:{submission_id}");
    let payload = match serde_json::to_string(event) {
        Ok(payload) => payload,
        Err(error) => {
            warn!(error = %error, "failed to serialize runtime event");
            return;
        }
    };

    match client.get_multiplexed_async_connection().await {
        Ok(mut connection) => {
            if let Err(error) = redis::cmd("PUBLISH")
                .arg(&channel)
                .arg(&payload)
                .query_async::<i64>(&mut connection)
                .await
            {
                warn!(channel = %channel, error = %error, "failed to publish runtime event");
            }
        }
        Err(error) => {
            warn!(error = %error, "failed to open redis connection");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        annotate_runtime_node_health, filter_runtime_nodes, summarize_runtime_nodes,
        RuntimeNodeRegistryQuery, RuntimeNodeRegistrySummary, RuntimeRouteCoverage,
    };
    use nexus_runtime::{
        RuntimeNodeHealthStatus, RuntimeNodeStatus, RuntimeRouteBinding, RuntimeWorkerGroup,
    };

    fn sample_nodes() -> Vec<RuntimeNodeStatus> {
        vec![
            RuntimeNodeStatus {
                node_id: "node-a".to_owned(),
                started_at_ms: 1,
                last_heartbeat_ms: 1_000,
                node_status: RuntimeNodeHealthStatus::Healthy,
                worker_groups: vec![
                    RuntimeWorkerGroup {
                        name: "oj-fast".to_owned(),
                        bindings: vec![RuntimeRouteBinding {
                            queue: "oj_judge".to_owned(),
                            lane: "fast".to_owned(),
                        }],
                    },
                    RuntimeWorkerGroup {
                        name: "oj-special".to_owned(),
                        bindings: vec![RuntimeRouteBinding {
                            queue: "oj_judge".to_owned(),
                            lane: "special".to_owned(),
                        }],
                    },
                ],
            },
            RuntimeNodeStatus {
                node_id: "node-b".to_owned(),
                started_at_ms: 2,
                last_heartbeat_ms: 2_000,
                node_status: RuntimeNodeHealthStatus::Healthy,
                worker_groups: vec![RuntimeWorkerGroup {
                    name: "oj-fast".to_owned(),
                    bindings: vec![RuntimeRouteBinding {
                        queue: "oj_judge".to_owned(),
                        lane: "fast".to_owned(),
                    }],
                }],
            },
        ]
    }

    #[test]
    fn node_registry_filter_supports_queue_lane_and_group() {
        let filtered = filter_runtime_nodes(
            sample_nodes(),
            &RuntimeNodeRegistryQuery {
                queue: Some("oj_judge".to_owned()),
                lane: Some("special".to_owned()),
                group: None,
                status: None,
            },
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].node_id, "node-a");

        let filtered = filter_runtime_nodes(
            sample_nodes(),
            &RuntimeNodeRegistryQuery {
                queue: None,
                lane: None,
                group: Some("oj-fast".to_owned()),
                status: None,
            },
        );
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn node_registry_filter_supports_status() {
        let filtered = filter_runtime_nodes(
            annotate_runtime_node_health(sample_nodes(), 25_000),
            &RuntimeNodeRegistryQuery {
                queue: None,
                lane: None,
                group: None,
                status: Some(RuntimeNodeHealthStatus::Stale),
            },
        );

        assert_eq!(filtered.len(), 2);

        let filtered = filter_runtime_nodes(
            annotate_runtime_node_health(sample_nodes(), 10_000),
            &RuntimeNodeRegistryQuery {
                queue: None,
                lane: None,
                group: None,
                status: Some(RuntimeNodeHealthStatus::Healthy),
            },
        );

        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn node_registry_health_annotation_marks_stale_nodes() {
        let nodes = annotate_runtime_node_health(sample_nodes(), 25_000);

        assert_eq!(nodes[0].node_status, RuntimeNodeHealthStatus::Stale);
        assert_eq!(nodes[1].node_status, RuntimeNodeHealthStatus::Stale);

        let fresh_nodes = annotate_runtime_node_health(sample_nodes(), 10_000);
        assert_eq!(fresh_nodes[0].node_status, RuntimeNodeHealthStatus::Healthy);
        assert_eq!(fresh_nodes[1].node_status, RuntimeNodeHealthStatus::Healthy);
    }

    #[test]
    fn node_registry_summary_aggregates_route_coverage() {
        let summary = summarize_runtime_nodes(&sample_nodes());

        assert_eq!(
            summary,
            RuntimeNodeRegistrySummary {
                total_nodes: 2,
                total_worker_groups: 3,
                healthy_nodes: 2,
                stale_nodes: 0,
                routes: vec![
                    RuntimeRouteCoverage {
                        queue: "oj_judge".to_owned(),
                        lane: "fast".to_owned(),
                        node_count: 2,
                        worker_group_count: 2,
                    },
                    RuntimeRouteCoverage {
                        queue: "oj_judge".to_owned(),
                        lane: "special".to_owned(),
                        node_count: 1,
                        worker_group_count: 1,
                    },
                ],
                groups: vec![
                    super::RuntimeGroupCoverage {
                        name: "oj-fast".to_owned(),
                        node_count: 2,
                        binding_count: 1,
                    },
                    super::RuntimeGroupCoverage {
                        name: "oj-special".to_owned(),
                        node_count: 1,
                        binding_count: 1,
                    },
                ],
            }
        );
    }
}
