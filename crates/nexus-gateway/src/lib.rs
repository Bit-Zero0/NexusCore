use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::Query,
    http::{header, Method},
    routing::get,
    Json, Router,
};
use nexus_auth::build_dev_auth_service;
use nexus_config::{
    AppConfig, OjRepositoryMode, RuntimeBrokerBackend as ConfigRuntimeBrokerBackend,
    RuntimeSeccompMode as ConfigRuntimeSeccompMode, RuntimeSyscallArch as ConfigRuntimeSyscallArch,
    RuntimeSyscallFlavor as ConfigRuntimeSyscallFlavor, RuntimeWorkerGroupConfig,
};
use nexus_jobs::{
    build_router as build_jobs_router, oj_judge_job_handler, DefaultJobQueryService,
    DefaultJobSubmissionValidator, InMemoryJobDefinitionStore, InMemoryJobEventStore,
    InMemoryJobHandlerRegistry, JobHandlerRegistry, JobPlatformService, JobQueryService,
    JobRuntimeEventObserver, RuntimeBackedJobSubmitter,
};
use nexus_oj::{
    build_default_catalog, build_router as build_oj_router, InMemoryProblemRepository,
    InMemorySubmissionRepository, OjService, PgProblemRepository, PgSubmissionRepository,
};
use nexus_runtime::{
    build_default_runtime_catalog, build_nats_runtime_queue, build_rabbitmq_runtime_queue,
    build_redis_streams_runtime_queue, build_router as build_runtime_router, build_runtime_queue,
    EnhancedBrokerCapabilities, NatsQueueConfig, RabbitMqQueueConfig, RedisStreamsQueueConfig,
    RequiredBrokerCapabilities, RuntimeBrokerBackend, RuntimeBrokerObservabilityStatus,
    RuntimeEventObserver, RuntimeNodeHealthStatus, RuntimeNodeStatus, RuntimeRouteBinding,
    RuntimeSeccompMode, RuntimeSyscallArch, RuntimeSyscallFlavor, RuntimeTaskEvent,
    RuntimeTaskService, RuntimeWorker, RuntimeWorkerGroup, MEMORY_BROKER_CAPABILITIES,
    NATS_BROKER_CAPABILITIES, RABBITMQ_BROKER_CAPABILITIES, REDIS_STREAMS_BROKER_CAPABILITIES,
};
use nexus_shared::HealthStatus;
use nexus_storage::PostgresPoolFactory;
use redis::Client as RedisClient;
use serde::{Deserialize, Serialize};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::{DefaultMakeSpan, TraceLayer},
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

pub struct GatewayServices {
    pub oj_service: Arc<OjService>,
    pub runtime_service: Arc<RuntimeTaskService>,
    pub job_platform_service: Arc<JobPlatformService>,
    pub job_query_service: Arc<dyn JobQueryService>,
    pub job_handler_registry: Arc<InMemoryJobHandlerRegistry>,
}

pub async fn build_router(config: &AppConfig) -> nexus_shared::AppResult<Router> {
    let redis_client = RedisClient::open(config.redis.url.as_str()).ok();
    let services = build_gateway_services(config, redis_client.clone()).await?;
    Ok(build_router_with_services(
        services,
        redis_client,
        false,
        &config.server.cors_allowed_origins,
    ))
}

pub fn build_router_with_services(
    services: GatewayServices,
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
        .route(
            "/metrics",
            get({
                let runtime_service = services.runtime_service.clone();
                move || render_metrics(runtime_service.clone())
            }),
        )
        .route("/api/v1/runtime/nodes", runtime_nodes_route)
        .route("/api/v1/runtime/nodes/summary", runtime_nodes_summary_route)
        .merge(nexus_auth::build_router(build_dev_auth_service()))
        .merge(build_jobs_router(
            services.job_query_service.clone(),
            services.job_handler_registry.clone(),
        ))
        .merge(build_oj_router(
            build_default_catalog(),
            services.oj_service.clone(),
            Some(services.job_platform_service.clone()),
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(false)),
        );

    let router = if expose_runtime_api {
        router.merge(build_runtime_router(services.runtime_service))
    } else {
        router
    };

    apply_cors_layer(router, cors_allowed_origins)
}

pub async fn build_gateway_services(
    config: &AppConfig,
    redis_client: Option<RedisClient>,
) -> nexus_shared::AppResult<GatewayServices> {
    let job_definition_store = Arc::new(InMemoryJobDefinitionStore::default());
    let job_event_store = Arc::new(InMemoryJobEventStore::default());
    let job_handler_registry = Arc::new(InMemoryJobHandlerRegistry::default());
    job_handler_registry.register_handler(oj_judge_job_handler());
    let (oj_service, runtime_observer) =
        build_oj_service_and_observer(config, redis_client, job_event_store.clone()).await?;
    let (runtime_queue, broker_status) = build_runtime_queue_from_config(config).await?;
    let runtime_service = Arc::new(RuntimeTaskService::with_queue_and_broker(
        Arc::new(RuntimeWorker::new(
            build_default_runtime_catalog(),
            config.runtime.work_root.clone(),
            config.runtime.nsjail_path.clone(),
            map_runtime_seccomp_mode(config.runtime.seccomp_mode),
            map_runtime_syscall_flavor(config.runtime.syscall_flavor),
            map_runtime_syscall_arch(config.runtime.syscall_arch),
        )),
        runtime_queue,
        broker_status,
        runtime_observer,
    ));

    let job_platform_service = Arc::new(JobPlatformService::new(
        Arc::new(RuntimeBackedJobSubmitter::new(
            runtime_service.clone(),
            job_handler_registry.clone(),
        )),
        Arc::new(DefaultJobSubmissionValidator::new(
            job_handler_registry.clone(),
        )),
        job_definition_store.clone(),
        job_event_store.clone(),
    ));
    let job_query_service = Arc::new(DefaultJobQueryService::new(
        runtime_service.clone(),
        job_definition_store,
        job_event_store,
        job_handler_registry.clone(),
    )) as Arc<dyn JobQueryService>;

    Ok(GatewayServices {
        oj_service,
        runtime_service,
        job_platform_service,
        job_query_service,
        job_handler_registry,
    })
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

fn map_runtime_syscall_arch(arch: ConfigRuntimeSyscallArch) -> RuntimeSyscallArch {
    match arch {
        ConfigRuntimeSyscallArch::Auto => RuntimeSyscallArch::Auto,
        ConfigRuntimeSyscallArch::X86_64 => RuntimeSyscallArch::X86_64,
        ConfigRuntimeSyscallArch::Aarch64 => RuntimeSyscallArch::Aarch64,
        ConfigRuntimeSyscallArch::Other => RuntimeSyscallArch::Other,
    }
}

async fn build_oj_service_and_observer(
    config: &AppConfig,
    redis_client: Option<RedisClient>,
    job_event_store: Arc<InMemoryJobEventStore>,
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
                Arc::new(CompositeRuntimeObserver::new(vec![
                    Arc::new(GatewayRuntimeObserver::new(oj_service, redis_client))
                        as Arc<dyn RuntimeEventObserver>,
                    Arc::new(JobRuntimeEventObserver::new(job_event_store))
                        as Arc<dyn RuntimeEventObserver>,
                ])) as Arc<dyn RuntimeEventObserver>,
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
                Arc::new(CompositeRuntimeObserver::new(vec![
                    Arc::new(GatewayRuntimeObserver::new(oj_service, redis_client))
                        as Arc<dyn RuntimeEventObserver>,
                    Arc::new(JobRuntimeEventObserver::new(job_event_store))
                        as Arc<dyn RuntimeEventObserver>,
                ])) as Arc<dyn RuntimeEventObserver>,
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

async fn render_metrics(
    runtime_service: Arc<RuntimeTaskService>,
) -> nexus_shared::AppResult<([(header::HeaderName, &'static str); 1], String)> {
    let body = nexus_runtime::render_prometheus_metrics(runtime_service.as_ref()).await?;
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body))
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

fn map_runtime_broker_backend(backend: ConfigRuntimeBrokerBackend) -> RuntimeBrokerBackend {
    match backend {
        ConfigRuntimeBrokerBackend::Memory => RuntimeBrokerBackend::Memory,
        ConfigRuntimeBrokerBackend::RabbitMq => RuntimeBrokerBackend::Memory,
        ConfigRuntimeBrokerBackend::Nats => RuntimeBrokerBackend::Memory,
        ConfigRuntimeBrokerBackend::RedisStreams => RuntimeBrokerBackend::Memory,
    }
}

async fn build_runtime_queue_from_config(
    config: &AppConfig,
) -> nexus_shared::AppResult<(
    Arc<dyn nexus_runtime::RuntimeTaskQueue>,
    RuntimeBrokerObservabilityStatus,
)> {
    match config.runtime.broker_backend {
        ConfigRuntimeBrokerBackend::Memory => {
            info!(broker = "memory", "building runtime broker");
            Ok((
                build_runtime_queue(map_runtime_broker_backend(config.runtime.broker_backend)),
                runtime_broker_status(
                    MEMORY_BROKER_CAPABILITIES.broker,
                    MEMORY_BROKER_CAPABILITIES.required,
                    MEMORY_BROKER_CAPABILITIES.enhanced,
                ),
            ))
        }
        ConfigRuntimeBrokerBackend::RabbitMq => {
            info!(
                broker = "rabbitmq",
                exchange = %config.runtime.rabbitmq.exchange,
                queue_prefix = %config.runtime.rabbitmq.queue_prefix,
                "building runtime broker"
            );
            Ok((
                build_rabbitmq_runtime_queue(RabbitMqQueueConfig {
                    url: config.runtime.rabbitmq.url.clone(),
                    exchange: config.runtime.rabbitmq.exchange.clone(),
                    queue_prefix: config.runtime.rabbitmq.queue_prefix.clone(),
                })
                .await?,
                runtime_broker_status(
                    RABBITMQ_BROKER_CAPABILITIES.broker,
                    RABBITMQ_BROKER_CAPABILITIES.required,
                    RABBITMQ_BROKER_CAPABILITIES.enhanced,
                ),
            ))
        }
        ConfigRuntimeBrokerBackend::Nats => {
            info!(
                broker = "nats",
                stream = %config.runtime.nats.stream_name,
                subject_prefix = %config.runtime.nats.subject_prefix,
                "building runtime broker"
            );
            Ok((
                build_nats_runtime_queue(NatsQueueConfig {
                    url: config.runtime.nats.url.clone(),
                    stream_name: config.runtime.nats.stream_name.clone(),
                    subject_prefix: config.runtime.nats.subject_prefix.clone(),
                    consumer_prefix: config.runtime.nats.consumer_prefix.clone(),
                    ack_wait_ms: config.runtime.nats.ack_wait_ms,
                })
                .await?,
                runtime_broker_status(
                    NATS_BROKER_CAPABILITIES.broker,
                    NATS_BROKER_CAPABILITIES.required,
                    NATS_BROKER_CAPABILITIES.enhanced,
                )
                .with_ack_wait_ms(Some(config.runtime.nats.ack_wait_ms)),
            ))
        }
        ConfigRuntimeBrokerBackend::RedisStreams => {
            info!(
                broker = "redis_streams",
                stream_prefix = %config.runtime.redis_streams.stream_prefix,
                consumer_group_prefix = %config.runtime.redis_streams.consumer_group_prefix,
                "building runtime broker"
            );
            Ok((
                build_redis_streams_runtime_queue(RedisStreamsQueueConfig {
                    url: config.runtime.redis_streams.url.clone(),
                    stream_prefix: config.runtime.redis_streams.stream_prefix.clone(),
                    consumer_group_prefix: config
                        .runtime
                        .redis_streams
                        .consumer_group_prefix
                        .clone(),
                    consumer_name_prefix: config.runtime.redis_streams.consumer_name_prefix.clone(),
                    pending_reclaim_idle_ms: config.runtime.redis_streams.pending_reclaim_idle_ms,
                })
                .await?,
                runtime_broker_status(
                    REDIS_STREAMS_BROKER_CAPABILITIES.broker,
                    REDIS_STREAMS_BROKER_CAPABILITIES.required,
                    REDIS_STREAMS_BROKER_CAPABILITIES.enhanced,
                )
                .with_pending_reclaim_idle_ms(Some(
                    config.runtime.redis_streams.pending_reclaim_idle_ms,
                )),
            ))
        }
    }
}

fn runtime_broker_status(
    broker: &str,
    required_capabilities: RequiredBrokerCapabilities,
    enhanced_capabilities: EnhancedBrokerCapabilities,
) -> RuntimeBrokerObservabilityStatus {
    RuntimeBrokerObservabilityStatus::from_capability_profile(
        broker,
        required_capabilities,
        enhanced_capabilities,
    )
}

struct GatewayRuntimeObserver {
    oj_service: Arc<OjService>,
    redis_client: Option<RedisClient>,
}

struct CompositeRuntimeObserver {
    observers: Vec<Arc<dyn RuntimeEventObserver>>,
}

impl CompositeRuntimeObserver {
    fn new(observers: Vec<Arc<dyn RuntimeEventObserver>>) -> Self {
        Self { observers }
    }
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
        info!(
            task_id = %event.task_id,
            queue = %event.queue,
            lane = %event.lane,
            status = ?event.status,
            submission_id = event.submission_id.as_deref().unwrap_or("-"),
            "gateway received runtime event"
        );
        self.oj_service.apply_runtime_event(&event).await?;

        if let (Some(submission_id), Some(redis_client)) =
            (&event.submission_id, &self.redis_client)
        {
            publish_runtime_event(redis_client, submission_id, &event).await;
        }

        Ok(())
    }
}

#[async_trait]
impl RuntimeEventObserver for CompositeRuntimeObserver {
    async fn on_event(&self, event: RuntimeTaskEvent) -> nexus_shared::AppResult<()> {
        for observer in &self.observers {
            observer.on_event(event.clone()).await?;
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
        RuntimeBrokerObservabilityStatus, RuntimeNodeHealthStatus, RuntimeNodeStatus,
        RuntimeRouteBinding, RuntimeWorkerGroup,
    };

    fn sample_nodes() -> Vec<RuntimeNodeStatus> {
        vec![
            RuntimeNodeStatus {
                node_id: "node-a".to_owned(),
                started_at_ms: 1,
                last_heartbeat_ms: 1_000,
                node_status: RuntimeNodeHealthStatus::Healthy,
                broker: RuntimeBrokerObservabilityStatus::memory(),
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
                broker: RuntimeBrokerObservabilityStatus::memory(),
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
