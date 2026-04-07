mod nats;
mod rabbitmq;
mod redis;

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use ::redis::{
    aio::MultiplexedConnection,
    streams::{
        StreamAutoClaimOptions, StreamAutoClaimReply, StreamInfoGroupsReply, StreamRangeReply,
        StreamReadOptions, StreamReadReply,
    },
    AsyncCommands, Client as RedisClient, RedisError,
};
use async_nats::{
    jetstream::{
        self,
        consumer::{self, pull},
        stream::{self, RetentionPolicy, StorageType},
        AckKind,
    },
    Client as NatsClient,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use lapin::{
    options::{
        BasicAckOptions, BasicGetOptions, BasicPublishOptions, ConfirmSelectOptions,
        ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions,
    },
    types::{AMQPValue, FieldTable},
    BasicProperties, Channel, Connection, ConnectionProperties, ExchangeKind,
};
use nexus_shared::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info};
use ulid::Ulid;

use crate::{
    executor::RetryDisposition, observe_broker_operation_failure, observe_broker_reclaim,
    observe_broker_reclaim_orphan_cleanup, observe_broker_replay, InMemoryRuntimeTaskQueue,
    RuntimeDeadLetterRecord, RuntimeQueueReceipt, RuntimeQueueStats, RuntimeRouteBinding,
    RuntimeTask, RuntimeTaskDelivery, RuntimeTaskQueue,
};

#[derive(Debug, Clone)]
pub struct RabbitMqQueueConfig {
    pub url: String,
    pub exchange: String,
    pub queue_prefix: String,
}

#[derive(Debug, Clone)]
pub struct NatsQueueConfig {
    pub url: String,
    pub stream_name: String,
    pub subject_prefix: String,
    pub consumer_prefix: String,
    pub ack_wait_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RedisStreamsQueueConfig {
    pub url: String,
    pub stream_prefix: String,
    pub consumer_group_prefix: String,
    pub consumer_name_prefix: String,
    pub pending_reclaim_idle_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredBrokerCapabilities {
    pub enqueue: bool,
    pub reserve: bool,
    pub ack: bool,
    pub retry: bool,
    pub reject: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnhancedBrokerCapabilities {
    pub stats: bool,
    pub dead_letter_store: bool,
    pub dead_letter_replay: bool,
    pub route_fairness: bool,
    pub crash_reclaim: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerCapabilityProfile {
    pub broker: &'static str,
    pub required: RequiredBrokerCapabilities,
    pub enhanced: EnhancedBrokerCapabilities,
}

pub const REQUIRED_BROKER_CAPABILITIES: RequiredBrokerCapabilities = RequiredBrokerCapabilities {
    enqueue: true,
    reserve: true,
    ack: true,
    retry: true,
    reject: true,
};

pub const MEMORY_BROKER_CAPABILITIES: BrokerCapabilityProfile = BrokerCapabilityProfile {
    broker: "memory",
    required: REQUIRED_BROKER_CAPABILITIES,
    enhanced: EnhancedBrokerCapabilities {
        stats: true,
        dead_letter_store: true,
        dead_letter_replay: true,
        route_fairness: true,
        crash_reclaim: false,
    },
};

pub const RABBITMQ_BROKER_CAPABILITIES: BrokerCapabilityProfile = BrokerCapabilityProfile {
    broker: "rabbitmq",
    required: REQUIRED_BROKER_CAPABILITIES,
    enhanced: EnhancedBrokerCapabilities {
        stats: true,
        dead_letter_store: true,
        dead_letter_replay: true,
        route_fairness: true,
        crash_reclaim: true,
    },
};

pub const NATS_BROKER_CAPABILITIES: BrokerCapabilityProfile = BrokerCapabilityProfile {
    broker: "nats",
    required: REQUIRED_BROKER_CAPABILITIES,
    enhanced: EnhancedBrokerCapabilities {
        stats: true,
        dead_letter_store: true,
        dead_letter_replay: true,
        route_fairness: true,
        crash_reclaim: true,
    },
};

pub const REDIS_STREAMS_BROKER_CAPABILITIES: BrokerCapabilityProfile = BrokerCapabilityProfile {
    broker: "redis_streams",
    required: REQUIRED_BROKER_CAPABILITIES,
    enhanced: EnhancedBrokerCapabilities {
        stats: true,
        dead_letter_store: true,
        dead_letter_replay: true,
        route_fairness: true,
        crash_reclaim: true,
    },
};

#[derive(Debug, Clone)]
pub struct RabbitMqRouteTopology {
    pub queue: String,
    pub lane: String,
    pub exchange: String,
    pub routing_key: String,
    pub queue_name: String,
    pub retry_exchange: String,
    pub retry_queue_name: String,
    pub retry_routing_key: String,
    pub dead_letter_exchange: String,
    pub dead_letter_queue_name: String,
    pub dead_letter_routing_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RabbitMqEnvelope {
    task: RuntimeTask,
    attempt: u32,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct RabbitMqLeasedDelivery {
    delivery_tag: u64,
    topology: RabbitMqRouteTopology,
    delivery: RuntimeTaskDelivery,
}

#[derive(Debug, Clone)]
struct RabbitMqDeadLetterMessage {
    delivery_tag: u64,
    record: RuntimeDeadLetterRecord,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct RabbitMqRouteRegistryEntry {
    queue: String,
    lane: String,
}

#[derive(Debug, Clone)]
struct NatsRouteTopology {
    queue: String,
    lane: String,
    subject: String,
    consumer_name: String,
    dead_letter_subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NatsEnvelope {
    task: RuntimeTask,
    last_error: Option<String>,
}

struct NatsLeasedDelivery {
    acker: async_nats::jetstream::message::Acker,
    delivery: RuntimeTaskDelivery,
}

#[derive(Debug, Clone)]
struct RedisRouteTopology {
    queue: String,
    lane: String,
    stream_key: String,
    consumer_group: String,
    dead_letter_stream_key: String,
    delayed_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RedisEnvelope {
    task: RuntimeTask,
    attempt: u32,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RedisDelayedEnvelope {
    token: String,
    envelope: RedisEnvelope,
}

#[derive(Debug, Clone)]
struct RedisLeasedDelivery {
    entry_id: String,
    topology: RedisRouteTopology,
    delivery: RuntimeTaskDelivery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct RedisRouteRegistryEntry {
    queue: String,
    lane: String,
}

struct RabbitMqBrokerAdapter {
    config: RabbitMqQueueConfig,
    broker_enabled: bool,
    inner: InMemoryRuntimeTaskQueue,
    transport: AsyncMutex<Option<Arc<RabbitMqTransport>>>,
    routes: RouteCatalog<RabbitMqRouteTopology>,
    leased: AsyncMutex<HashMap<String, RabbitMqLeasedDelivery>>,
}

struct RabbitMqTransport {
    connection: Connection,
    channel: Channel,
}

struct NatsBrokerAdapter {
    config: NatsQueueConfig,
    transport: NatsTransport,
    routes: RouteCatalog<NatsRouteTopology>,
    leased: AsyncMutex<HashMap<String, NatsLeasedDelivery>>,
}

struct RedisStreamsBrokerAdapter {
    config: RedisStreamsQueueConfig,
    inner: InMemoryRuntimeTaskQueue,
    transport: Option<RedisStreamsTransport>,
    routes: RouteCatalog<RedisRouteTopology>,
    leased: AsyncMutex<HashMap<String, RedisLeasedDelivery>>,
}

struct NatsTransport {
    _client: NatsClient,
    jetstream: jetstream::Context,
    work_stream: stream::Stream,
    dead_letter_stream: stream::Stream,
    ack_wait_ms: u64,
}

struct RedisStreamsTransport {
    client: RedisClient,
    consumer_name: String,
}

struct DeadLetterReplay<TTopology, TStored> {
    topology: TTopology,
    stored: TStored,
    record: RuntimeDeadLetterRecord,
}

struct RouteCatalog<T> {
    declared_routes: Mutex<BTreeMap<(String, String), T>>,
    route_order: Mutex<VecDeque<(String, String)>>,
}

impl<T> RouteCatalog<T> {
    fn new() -> Self {
        Self {
            declared_routes: Mutex::new(BTreeMap::new()),
            route_order: Mutex::new(VecDeque::new()),
        }
    }
}

impl<T> RouteCatalog<T>
where
    T: Clone,
{
    fn get_or_insert_with<F>(&self, queue: &str, lane: &str, build: F) -> AppResult<T>
    where
        F: FnOnce() -> T,
    {
        let key = (queue.to_owned(), lane.to_owned());
        let mut declared_routes = self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?;
        if let Some(topology) = declared_routes.get(&key) {
            return Ok(topology.clone());
        }

        let topology = build();
        declared_routes.insert(key.clone(), topology.clone());
        drop(declared_routes);

        self.route_order
            .lock()
            .map_err(|_| AppError::Internal)?
            .push_back(key);

        Ok(topology)
    }

    fn get(&self, route: &(String, String)) -> AppResult<Option<T>> {
        Ok(self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?
            .get(route)
            .cloned())
    }

    fn topologies(&self) -> AppResult<Vec<T>> {
        Ok(self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?
            .values()
            .cloned()
            .collect())
    }

    fn ordered_routes(
        &self,
        bindings: &[RuntimeRouteBinding],
    ) -> AppResult<VecDeque<(String, String)>> {
        Ok(self
            .route_order
            .lock()
            .map_err(|_| AppError::Internal)?
            .iter()
            .filter(|route| {
                bindings.is_empty()
                    || bindings
                        .iter()
                        .any(|binding| binding.queue == route.0 && binding.lane == route.1)
            })
            .cloned()
            .collect())
    }

    fn rotate(&self, route: &(String, String)) {
        if let Ok(mut order) = self.route_order.lock() {
            if let Some(position) = order.iter().position(|item| item == route) {
                order.remove(position);
                order.push_back(route.clone());
            }
        }
    }

    fn register_route(&self, queue: &str, lane: &str, topology: T) -> AppResult<()> {
        let route = (queue.to_owned(), lane.to_owned());
        self.declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?
            .entry(route.clone())
            .or_insert(topology);
        let mut order = self.route_order.lock().map_err(|_| AppError::Internal)?;
        if !order.contains(&route) {
            order.push_back(route);
        }
        Ok(())
    }
}

#[async_trait]
trait BrokerAdapter: Send + Sync {
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()>;
    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery>;
    async fn ack(&self, delivery_id: &str) -> AppResult<()>;
    async fn retry(
        &self,
        delivery_id: &str,
        error: &str,
        delay_ms: u64,
    ) -> AppResult<RetryDisposition>;
    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()>;
    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>>;
    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>>;
    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt>;
}

#[async_trait]
trait BrokerDeadLetterStore: Send + Sync {
    type Topology: Clone + Send + Sync;
    type Stored: Send + Sync;

    async fn store_dead_letter(
        &self,
        topology: &Self::Topology,
        record: &RuntimeDeadLetterRecord,
    ) -> AppResult<()>;
    async fn load_dead_letter_records(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>>;
    async fn find_dead_letter_for_replay(
        &self,
        delivery_id: &str,
    ) -> AppResult<Option<DeadLetterReplay<Self::Topology, Self::Stored>>>;
    async fn remove_replayed_dead_letter(
        &self,
        replay: DeadLetterReplay<Self::Topology, Self::Stored>,
    ) -> AppResult<()>;
}

struct AdapterBackedRuntimeTaskQueue<A> {
    adapter: A,
}

impl<A> AdapterBackedRuntimeTaskQueue<A> {
    fn from_adapter(adapter: A) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl<A> RuntimeTaskQueue for AdapterBackedRuntimeTaskQueue<A>
where
    A: BrokerAdapter,
{
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()> {
        self.adapter.enqueue(task).await
    }

    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery> {
        self.adapter.reserve(bindings).await
    }

    async fn ack(&self, delivery_id: &str) -> AppResult<()> {
        self.adapter.ack(delivery_id).await
    }

    async fn retry(
        &self,
        delivery_id: &str,
        error: &str,
        delay_ms: u64,
    ) -> AppResult<RetryDisposition> {
        self.adapter.retry(delivery_id, error, delay_ms).await
    }

    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()> {
        self.adapter.reject(delivery_id, error).await
    }

    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        self.adapter.stats().await
    }

    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        self.adapter.dead_letters().await
    }

    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        self.adapter.replay_dead_letter(delivery_id).await
    }
}

type RabbitMqRuntimeTaskQueue = AdapterBackedRuntimeTaskQueue<RabbitMqBrokerAdapter>;
type NatsRuntimeTaskQueue = AdapterBackedRuntimeTaskQueue<NatsBrokerAdapter>;
type RedisStreamsRuntimeTaskQueue = AdapterBackedRuntimeTaskQueue<RedisStreamsBrokerAdapter>;

impl AdapterBackedRuntimeTaskQueue<RabbitMqBrokerAdapter> {
    #[cfg(test)]
    fn new(config: RabbitMqQueueConfig) -> AppResult<Self> {
        Ok(Self::from_adapter(RabbitMqBrokerAdapter::new(config)?))
    }

    #[cfg(test)]
    fn declared_topologies(&self) -> AppResult<Vec<RabbitMqRouteTopology>> {
        self.adapter.declared_topologies()
    }
}

pub async fn build_rabbitmq_runtime_queue(
    config: RabbitMqQueueConfig,
) -> AppResult<Arc<dyn RuntimeTaskQueue>> {
    let adapter = RabbitMqBrokerAdapter::connect(config).await?;
    let capabilities = RABBITMQ_BROKER_CAPABILITIES;
    info!(
        broker = %capabilities.broker,
        exchange = %adapter.config.exchange,
        queue_prefix = %adapter.config.queue_prefix,
        crash_reclaim = capabilities.enhanced.crash_reclaim,
        dead_letter_replay = capabilities.enhanced.dead_letter_replay,
        "connecting runtime broker"
    );
    Ok(Arc::new(RabbitMqRuntimeTaskQueue::from_adapter(adapter)))
}

pub async fn build_nats_runtime_queue(
    config: NatsQueueConfig,
) -> AppResult<Arc<dyn RuntimeTaskQueue>> {
    let adapter = NatsBrokerAdapter::connect(config).await?;
    let capabilities = NATS_BROKER_CAPABILITIES;
    info!(
        broker = %capabilities.broker,
        stream = %adapter.config.stream_name,
        subject_prefix = %adapter.config.subject_prefix,
        consumer_prefix = %adapter.config.consumer_prefix,
        ack_wait_ms = adapter.config.ack_wait_ms,
        crash_reclaim = capabilities.enhanced.crash_reclaim,
        dead_letter_replay = capabilities.enhanced.dead_letter_replay,
        "connecting runtime broker"
    );
    Ok(Arc::new(NatsRuntimeTaskQueue::from_adapter(adapter)))
}

pub async fn build_redis_streams_runtime_queue(
    config: RedisStreamsQueueConfig,
) -> AppResult<Arc<dyn RuntimeTaskQueue>> {
    let adapter = RedisStreamsBrokerAdapter::connect(config).await?;
    let capabilities = REDIS_STREAMS_BROKER_CAPABILITIES;
    info!(
        broker = %capabilities.broker,
        stream_prefix = %adapter.config.stream_prefix,
        consumer_group_prefix = %adapter.config.consumer_group_prefix,
        consumer_name_prefix = %adapter.config.consumer_name_prefix,
        pending_reclaim_idle_ms = adapter.config.pending_reclaim_idle_ms,
        crash_reclaim = capabilities.enhanced.crash_reclaim,
        dead_letter_replay = capabilities.enhanced.dead_letter_replay,
        "connecting runtime broker"
    );
    Ok(Arc::new(RedisStreamsRuntimeTaskQueue::from_adapter(
        adapter,
    )))
}

fn build_route_topology(
    config: &RabbitMqQueueConfig,
    queue: &str,
    lane: &str,
) -> RabbitMqRouteTopology {
    let queue_part = sanitize_segment(queue);
    let lane_part = sanitize_segment(lane);
    let route_name = format!("{}.{}", queue_part, lane_part);
    let retry_exchange = format!("{}.retry", config.exchange);
    let dead_letter_exchange = format!("{}.dead", config.exchange);
    let routing_key = route_name.clone();
    let retry_routing_key = format!("{route_name}.retry");
    let dead_letter_routing_key = format!("{route_name}.dead");

    RabbitMqRouteTopology {
        queue: queue.to_owned(),
        lane: lane.to_owned(),
        exchange: config.exchange.clone(),
        routing_key: routing_key.clone(),
        queue_name: format!("{}.{}", config.queue_prefix, route_name),
        retry_exchange,
        retry_queue_name: format!("{}.{}.retry", config.queue_prefix, route_name),
        retry_routing_key,
        dead_letter_exchange,
        dead_letter_queue_name: format!("{}.{}.dead", config.queue_prefix, route_name),
        dead_letter_routing_key,
    }
}

fn build_nats_route_topology(
    config: &NatsQueueConfig,
    queue: &str,
    lane: &str,
) -> NatsRouteTopology {
    let queue_part = sanitize_segment(queue);
    let lane_part = sanitize_segment(lane);
    let route_name = format!("{queue_part}.{lane_part}");
    let dead_letter_prefix = nats_dead_letter_subject_prefix(config);

    NatsRouteTopology {
        queue: queue.to_owned(),
        lane: lane.to_owned(),
        subject: format!("{}.{}", config.subject_prefix, route_name),
        consumer_name: format!(
            "{}_{}",
            sanitize_consumer_name(&config.consumer_prefix),
            sanitize_consumer_name(&route_name)
        ),
        dead_letter_subject: format!("{dead_letter_prefix}.{route_name}"),
    }
}

fn build_redis_route_topology(
    config: &RedisStreamsQueueConfig,
    queue: &str,
    lane: &str,
) -> RedisRouteTopology {
    let queue_part = sanitize_segment(queue);
    let lane_part = sanitize_segment(lane);
    let route_name = format!("{queue_part}.{lane_part}");

    RedisRouteTopology {
        queue: queue.to_owned(),
        lane: lane.to_owned(),
        stream_key: format!("{}.{}", config.stream_prefix, route_name),
        consumer_group: format!(
            "{}:{}",
            sanitize_consumer_name(&config.consumer_group_prefix),
            sanitize_consumer_name(&route_name)
        ),
        dead_letter_stream_key: format!("{}.{}.dead", config.stream_prefix, route_name),
        delayed_key: format!("{}.{}.retry", config.stream_prefix, route_name),
    }
}

fn build_dead_letter_record(
    delivery: &RuntimeTaskDelivery,
    error: &str,
) -> RuntimeDeadLetterRecord {
    RuntimeDeadLetterRecord {
        delivery_id: delivery.delivery_id.clone(),
        task_id: delivery.task.task_id.clone(),
        queue: delivery.task.queue.clone(),
        lane: delivery.task.lane.clone(),
        attempt: delivery.attempt,
        error: error.to_owned(),
        dead_lettered_at: now_ms(),
        task: delivery.task.clone(),
    }
}

fn build_rabbitmq_replay_envelope(record: &RuntimeDeadLetterRecord) -> RabbitMqEnvelope {
    RabbitMqEnvelope {
        task: record.task.clone(),
        attempt: 1,
        last_error: Some(format!("replayed from dead letter: {}", record.error)),
    }
}

fn build_nats_replay_envelope(record: &RuntimeDeadLetterRecord) -> NatsEnvelope {
    NatsEnvelope {
        task: record.task.clone(),
        last_error: Some(format!("replayed from dead letter: {}", record.error)),
    }
}

fn build_replay_receipt(record: &RuntimeDeadLetterRecord) -> RuntimeQueueReceipt {
    RuntimeQueueReceipt {
        task_id: record.task.task_id.clone(),
        queue: record.task.queue.clone(),
        lane: record.task.lane.clone(),
        status: crate::RuntimeTaskLifecycleStatus::Queued,
    }
}

fn dead_letter_not_found(delivery_id: &str) -> AppError {
    AppError::NotFound(format!("dead letter not found: {delivery_id}"))
}

async fn replay_dead_letter_with_store<S, Publish, PublishFuture>(
    broker: &str,
    store: &S,
    delivery_id: &str,
    publish_replay: Publish,
) -> AppResult<RuntimeQueueReceipt>
where
    S: BrokerDeadLetterStore + Sync,
    Publish: Fn(S::Topology, RuntimeDeadLetterRecord) -> PublishFuture,
    PublishFuture: Future<Output = AppResult<()>>,
{
    let Some(replay) = store.find_dead_letter_for_replay(delivery_id).await? else {
        observe_broker_operation_failure(broker, "*", "*", "replay");
        return Err(dead_letter_not_found(delivery_id));
    };

    let receipt = build_replay_receipt(&replay.record);
    let topology = replay.topology.clone();
    let record = replay.record.clone();
    if let Err(error) = publish_replay(topology, record).await {
        observe_broker_operation_failure(
            broker,
            receipt.queue.as_str(),
            receipt.lane.as_str(),
            "replay",
        );
        return Err(error);
    }
    if let Err(error) = store.remove_replayed_dead_letter(replay).await {
        observe_broker_operation_failure(
            broker,
            receipt.queue.as_str(),
            receipt.lane.as_str(),
            "replay",
        );
        return Err(error);
    }
    observe_broker_replay(broker, receipt.queue.as_str(), receipt.lane.as_str());
    Ok(receipt)
}

fn rabbitmq_route_registry_queue_name(config: &RabbitMqQueueConfig) -> String {
    format!("{}.__routes", config.queue_prefix)
}

fn redis_route_registry_key(config: &RedisStreamsQueueConfig) -> String {
    format!("{}.__routes", config.stream_prefix)
}

fn nats_dead_letter_stream_name(config: &NatsQueueConfig) -> String {
    format!("{}_DLQ", config.stream_name)
}

fn nats_dead_letter_subject_prefix(config: &NatsQueueConfig) -> String {
    format!("dlq.{}", config.subject_prefix)
}

fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_consumer_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn rabbitmq_error(message: String) -> AppError {
    AppError::Database(message)
}

fn nats_error(message: String) -> AppError {
    AppError::Database(message)
}

fn redis_error(message: String) -> AppError {
    AppError::Database(message)
}

fn redis_conflict_error(error: &RedisError) -> bool {
    error.code() == Some("BUSYGROUP")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{build_route_topology, RabbitMqQueueConfig, RabbitMqRuntimeTaskQueue};
    use crate::{
        executor::RetryDisposition, OjJudgeTask, RuntimeJudgeMode, RuntimeLimits,
        RuntimeRetryPolicy, RuntimeSandboxKind, RuntimeTask, RuntimeTaskPayload, RuntimeTaskQueue,
        RuntimeTaskType, RuntimeTestcase,
    };
    use nexus_shared::{ProblemId, SubmissionId, UserId};

    #[tokio::test]
    async fn rabbitmq_queue_declares_topology_on_enqueue() {
        let queue = RabbitMqRuntimeTaskQueue::new(config()).expect("config should be valid");
        queue
            .enqueue(runtime_task("task-1", "oj_judge", "special"))
            .await
            .expect("enqueue should succeed");

        let topologies = queue
            .declared_topologies()
            .expect("topologies should be readable");
        assert_eq!(topologies.len(), 1);
        assert_eq!(topologies[0].queue_name, "nexus.runtime.oj_judge.special");
        assert_eq!(
            topologies[0].retry_queue_name,
            "nexus.runtime.oj_judge.special.retry"
        );
        assert_eq!(
            topologies[0].dead_letter_queue_name,
            "nexus.runtime.oj_judge.special.dead"
        );
    }

    #[tokio::test]
    async fn rabbitmq_emulated_queue_retry_requeues_delivery() {
        let queue = RabbitMqRuntimeTaskQueue::new(config()).expect("config should be valid");
        queue
            .enqueue(runtime_task("task-2", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");

        let delivery = queue.reserve(&[]).await.expect("delivery should exist");
        let disposition = queue
            .retry(&delivery.delivery_id, "temporary failure", 0)
            .await
            .expect("retry should succeed");

        assert_eq!(disposition, RetryDisposition::Requeued);
        let replayed = queue
            .reserve(&[])
            .await
            .expect("delivery should be requeued");
        assert_eq!(replayed.attempt, 2);
        assert_eq!(replayed.last_error.as_deref(), Some("temporary failure"));
    }

    #[test]
    fn route_topology_uses_expected_routing_keys() {
        let topology = build_route_topology(&config(), "oj_judge", "fast");
        assert_eq!(topology.exchange, "nexus.runtime");
        assert_eq!(topology.routing_key, "oj_judge.fast");
        assert_eq!(topology.retry_exchange, "nexus.runtime.retry");
        assert_eq!(topology.dead_letter_exchange, "nexus.runtime.dead");
        assert_eq!(topology.dead_letter_routing_key, "oj_judge.fast.dead");
    }

    fn config() -> RabbitMqQueueConfig {
        RabbitMqQueueConfig {
            url: "amqp://guest:guest@127.0.0.1:5672/%2f".to_owned(),
            exchange: "nexus.runtime".to_owned(),
            queue_prefix: "nexus.runtime".to_owned(),
        }
    }

    fn runtime_task(task_id: &str, queue: &str, lane: &str) -> RuntimeTask {
        RuntimeTask {
            task_id: task_id.to_owned(),
            task_type: RuntimeTaskType::OjJudge,
            source_domain: "oj".to_owned(),
            source_entity_id: format!("sub-{task_id}"),
            queue: queue.to_owned(),
            lane: lane.to_owned(),
            retry_policy: RuntimeRetryPolicy {
                max_attempts: 3,
                retry_delay_ms: 1000,
            },
            payload: RuntimeTaskPayload::OjJudge(OjJudgeTask {
                submission_id: SubmissionId::from(format!("sub-{task_id}")),
                problem_id: ProblemId::from("p-1"),
                user_id: UserId::from("u-1"),
                language: "cpp".to_owned(),
                judge_mode: RuntimeJudgeMode::Acm,
                sandbox_kind: RuntimeSandboxKind::Nsjail,
                source_code: "int main() { return 0; }".to_owned(),
                limits: RuntimeLimits {
                    time_limit_ms: 1000,
                    memory_limit_kb: 262144,
                },
                testcases: vec![RuntimeTestcase {
                    case_no: 1,
                    input: "1\n".to_owned(),
                    expected_output: "1\n".to_owned(),
                    score: 100,
                }],
                judge_config: None,
            }),
        }
    }
}
