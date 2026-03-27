use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use async_trait::async_trait;
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

use crate::{
    executor::RetryDisposition, InMemoryRuntimeTaskQueue, RuntimeDeadLetterRecord,
    RuntimeQueueReceipt, RuntimeQueueStats, RuntimeRouteBinding, RuntimeTask, RuntimeTaskDelivery,
    RuntimeTaskQueue,
};

#[derive(Debug, Clone)]
pub struct RabbitMqQueueConfig {
    pub url: String,
    pub exchange: String,
    pub queue_prefix: String,
}

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

pub async fn build_rabbitmq_runtime_queue(
    config: RabbitMqQueueConfig,
) -> AppResult<Arc<dyn RuntimeTaskQueue>> {
    let queue = RabbitMqRuntimeTaskQueue::connect(config).await?;
    Ok(Arc::new(queue))
}

pub struct RabbitMqRuntimeTaskQueue {
    config: RabbitMqQueueConfig,
    inner: InMemoryRuntimeTaskQueue,
    transport: Option<RabbitMqTransport>,
    declared_routes: Mutex<BTreeMap<(String, String), RabbitMqRouteTopology>>,
    route_order: Mutex<VecDeque<(String, String)>>,
    leased: AsyncMutex<HashMap<String, RabbitMqLeasedDelivery>>,
    dead_letters: AsyncMutex<Vec<RuntimeDeadLetterRecord>>,
}

struct RabbitMqTransport {
    _connection: Connection,
    channel: Channel,
}

impl RabbitMqRuntimeTaskQueue {
    #[cfg(test)]
    pub fn new(config: RabbitMqQueueConfig) -> AppResult<Self> {
        validate_rabbitmq_config(&config)?;
        Ok(Self {
            config,
            inner: InMemoryRuntimeTaskQueue::default(),
            transport: None,
            declared_routes: Mutex::new(BTreeMap::new()),
            route_order: Mutex::new(VecDeque::new()),
            leased: AsyncMutex::new(HashMap::new()),
            dead_letters: AsyncMutex::new(Vec::new()),
        })
    }

    pub async fn connect(config: RabbitMqQueueConfig) -> AppResult<Self> {
        validate_rabbitmq_config(&config)?;
        let connection = Connection::connect(&config.url, rabbitmq_connection_properties())
            .await
            .map_err(|error| rabbitmq_error(format!("failed to connect rabbitmq: {error}")))?;
        let channel = connection.create_channel().await.map_err(|error| {
            rabbitmq_error(format!("failed to create rabbitmq channel: {error}"))
        })?;
        channel
            .confirm_select(ConfirmSelectOptions::default())
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to enable rabbitmq publisher confirms: {error}"
                ))
            })?;

        Ok(Self {
            config,
            inner: InMemoryRuntimeTaskQueue::default(),
            transport: Some(RabbitMqTransport {
                _connection: connection,
                channel,
            }),
            declared_routes: Mutex::new(BTreeMap::new()),
            route_order: Mutex::new(VecDeque::new()),
            leased: AsyncMutex::new(HashMap::new()),
            dead_letters: AsyncMutex::new(Vec::new()),
        })
    }

    #[cfg(test)]
    pub fn declared_topologies(&self) -> AppResult<Vec<RabbitMqRouteTopology>> {
        Ok(self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?
            .values()
            .cloned()
            .collect())
    }

    fn ensure_route_topology(&self, task: &RuntimeTask) -> AppResult<RabbitMqRouteTopology> {
        let key = (task.queue.clone(), task.lane.clone());
        let mut declared_routes = self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?;
        if let Some(topology) = declared_routes.get(&key) {
            return Ok(topology.clone());
        }

        let topology = build_route_topology(&self.config, &task.queue, &task.lane);
        declared_routes.insert(key.clone(), topology.clone());
        drop(declared_routes);

        self.route_order
            .lock()
            .map_err(|_| AppError::Internal)?
            .push_back(key);

        Ok(topology)
    }

    fn topology_for_route(
        &self,
        route: &(String, String),
    ) -> AppResult<Option<RabbitMqRouteTopology>> {
        Ok(self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?
            .get(route)
            .cloned())
    }

    fn rotate_route(&self, route: &(String, String)) {
        if let Ok(mut order) = self.route_order.lock() {
            if let Some(position) = order.iter().position(|item| item == route) {
                order.remove(position);
                order.push_back(route.clone());
            }
        }
    }

    async fn publish_envelope(
        &self,
        topology: &RabbitMqRouteTopology,
        exchange: &str,
        routing_key: &str,
        envelope: &RabbitMqEnvelope,
        expiration_ms: Option<u64>,
    ) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };

        transport.ensure_topology(topology).await?;

        let payload = serde_json::to_vec(envelope).map_err(|_| AppError::Internal)?;
        let mut properties = BasicProperties::default()
            .with_content_type("application/json".into())
            .with_delivery_mode(2);
        if let Some(expiration_ms) = expiration_ms {
            properties = properties.with_expiration(expiration_ms.to_string().into());
        }

        let confirm = transport
            .channel
            .basic_publish(
                exchange,
                routing_key,
                BasicPublishOptions::default(),
                &payload,
                properties,
            )
            .await
            .map_err(|error| rabbitmq_error(format!("failed to publish runtime task: {error}")))?;
        confirm.await.map_err(|error| {
            rabbitmq_error(format!(
                "rabbitmq did not confirm runtime task publish: {error}"
            ))
        })?;
        Ok(())
    }

    async fn reserve_from_broker(
        &self,
        bindings: &[RuntimeRouteBinding],
    ) -> AppResult<Option<RuntimeTaskDelivery>> {
        let Some(transport) = &self.transport else {
            return Ok(None);
        };
        let routes_snapshot: VecDeque<_> = self
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
            .collect();

        for route in routes_snapshot {
            let Some(topology) = self.topology_for_route(&route)? else {
                continue;
            };

            let delivery = transport
                .channel
                .basic_get(&topology.queue_name, BasicGetOptions { no_ack: false })
                .await
                .map_err(|error| {
                    rabbitmq_error(format!(
                        "failed to reserve runtime task from {}: {error}",
                        topology.queue_name
                    ))
                })?;

            let Some(delivery) = delivery else {
                continue;
            };

            self.rotate_route(&route);

            let envelope: RabbitMqEnvelope =
                serde_json::from_slice(&delivery.data).map_err(|_| AppError::Internal)?;
            let delivery_id = format!("rmq-{}-{}", envelope.task.task_id, delivery.delivery_tag);
            let runtime_delivery = RuntimeTaskDelivery {
                delivery_id: delivery_id.clone(),
                attempt: envelope.attempt,
                leased_until: None,
                last_error: envelope.last_error.clone(),
                task: envelope.task.clone(),
            };

            self.leased.lock().await.insert(
                delivery_id,
                RabbitMqLeasedDelivery {
                    delivery_tag: delivery.delivery_tag,
                    topology,
                    delivery: runtime_delivery.clone(),
                },
            );
            return Ok(Some(runtime_delivery));
        }

        Ok(None)
    }

    async fn ack_broker_delivery(&self, delivery_id: &str) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let leased = self.leased.lock().await.remove(delivery_id);
        if let Some(leased) = leased {
            transport
                .channel
                .basic_ack(leased.delivery_tag, BasicAckOptions::default())
                .await
                .map_err(|error| {
                    rabbitmq_error(format!(
                        "failed to ack rabbitmq delivery {}: {error}",
                        leased.delivery_tag
                    ))
                })?;
        }
        Ok(())
    }

    async fn push_dead_letter_record(
        &self,
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
}

impl RabbitMqTransport {
    async fn ensure_topology(&self, topology: &RabbitMqRouteTopology) -> AppResult<()> {
        self.channel
            .exchange_declare(
                &topology.exchange,
                ExchangeKind::Direct,
                ExchangeDeclareOptions {
                    durable: true,
                    ..ExchangeDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to declare rabbitmq exchange {}: {error}",
                    topology.exchange
                ))
            })?;

        self.channel
            .exchange_declare(
                &topology.retry_exchange,
                ExchangeKind::Direct,
                ExchangeDeclareOptions {
                    durable: true,
                    ..ExchangeDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to declare rabbitmq retry exchange {}: {error}",
                    topology.retry_exchange
                ))
            })?;

        self.channel
            .exchange_declare(
                &topology.dead_letter_exchange,
                ExchangeKind::Direct,
                ExchangeDeclareOptions {
                    durable: true,
                    ..ExchangeDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to declare rabbitmq dead-letter exchange {}: {error}",
                    topology.dead_letter_exchange
                ))
            })?;

        self.channel
            .queue_declare(
                &topology.queue_name,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to declare rabbitmq queue {}: {error}",
                    topology.queue_name
                ))
            })?;

        self.channel
            .queue_bind(
                &topology.queue_name,
                &topology.exchange,
                &topology.routing_key,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to bind rabbitmq queue {}: {error}",
                    topology.queue_name
                ))
            })?;

        let mut retry_args = FieldTable::default();
        retry_args.insert(
            "x-dead-letter-exchange".into(),
            AMQPValue::LongString(topology.exchange.clone().into()),
        );
        retry_args.insert(
            "x-dead-letter-routing-key".into(),
            AMQPValue::LongString(topology.routing_key.clone().into()),
        );

        self.channel
            .queue_declare(
                &topology.retry_queue_name,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                retry_args,
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to declare rabbitmq retry queue {}: {error}",
                    topology.retry_queue_name
                ))
            })?;

        self.channel
            .queue_bind(
                &topology.retry_queue_name,
                &topology.retry_exchange,
                &topology.retry_routing_key,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to bind rabbitmq retry queue {}: {error}",
                    topology.retry_queue_name
                ))
            })?;

        self.channel
            .queue_declare(
                &topology.dead_letter_queue_name,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to declare rabbitmq dead-letter queue {}: {error}",
                    topology.dead_letter_queue_name
                ))
            })?;

        self.channel
            .queue_bind(
                &topology.dead_letter_queue_name,
                &topology.dead_letter_exchange,
                &topology.dead_letter_routing_key,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|error| {
                rabbitmq_error(format!(
                    "failed to bind rabbitmq dead-letter queue {}: {error}",
                    topology.dead_letter_queue_name
                ))
            })?;

        Ok(())
    }
}

#[async_trait]
impl RuntimeTaskQueue for RabbitMqRuntimeTaskQueue {
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()> {
        let topology = self.ensure_route_topology(&task)?;
        if self.transport.is_none() {
            return self.inner.enqueue(task).await;
        }

        self.publish_envelope(
            &topology,
            &topology.exchange,
            &topology.routing_key,
            &RabbitMqEnvelope {
                task,
                attempt: 1,
                last_error: None,
            },
            None,
        )
        .await
    }

    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery> {
        if self.transport.is_none() {
            return self.inner.reserve(bindings).await;
        }

        self.reserve_from_broker(bindings).await.ok().flatten()
    }

    async fn ack(&self, delivery_id: &str) -> AppResult<()> {
        if self.transport.is_none() {
            return self.inner.ack(delivery_id).await;
        }

        self.ack_broker_delivery(delivery_id).await
    }

    async fn retry(
        &self,
        delivery_id: &str,
        error: &str,
        delay_ms: u64,
    ) -> AppResult<RetryDisposition> {
        if self.transport.is_none() {
            return self.inner.retry(delivery_id, error, delay_ms).await;
        }

        let Some(leased) = self.leased.lock().await.remove(delivery_id) else {
            return Err(AppError::Internal);
        };

        if leased.delivery.attempt >= leased.delivery.task.retry_policy.max_attempts {
            let record = self.push_dead_letter_record(&leased.delivery, error).await;
            self.dead_letters.lock().await.push(record.clone());
            self.publish_envelope(
                &leased.topology,
                &leased.topology.dead_letter_exchange,
                &leased.topology.dead_letter_routing_key,
                &RabbitMqEnvelope {
                    task: record.task.clone(),
                    attempt: record.attempt,
                    last_error: Some(record.error.clone()),
                },
                None,
            )
            .await?;
            self.ack_broker_delivery(&leased.delivery.delivery_id)
                .await?;
            return Ok(RetryDisposition::DeadLettered);
        }

        self.publish_envelope(
            &leased.topology,
            &leased.topology.retry_exchange,
            &leased.topology.retry_routing_key,
            &RabbitMqEnvelope {
                task: leased.delivery.task.clone(),
                attempt: leased.delivery.attempt + 1,
                last_error: Some(error.to_owned()),
            },
            Some(delay_ms),
        )
        .await?;
        self.ack_broker_delivery(&leased.delivery.delivery_id)
            .await?;
        Ok(RetryDisposition::Requeued)
    }

    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()> {
        if self.transport.is_none() {
            return self.inner.reject(delivery_id, error).await;
        }

        let Some(leased) = self.leased.lock().await.remove(delivery_id) else {
            return Ok(());
        };
        let record = self.push_dead_letter_record(&leased.delivery, error).await;
        self.dead_letters.lock().await.push(record.clone());
        self.publish_envelope(
            &leased.topology,
            &leased.topology.dead_letter_exchange,
            &leased.topology.dead_letter_routing_key,
            &RabbitMqEnvelope {
                task: record.task.clone(),
                attempt: record.attempt,
                last_error: Some(record.error.clone()),
            },
            None,
        )
        .await?;
        self.ack_broker_delivery(&leased.delivery.delivery_id).await
    }

    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        if self.transport.is_none() {
            return self.inner.stats().await;
        }
        let Some(transport) = &self.transport else {
            return Ok(Vec::new());
        };

        let routes: Vec<_> = self
            .declared_routes
            .lock()
            .map_err(|_| AppError::Internal)?
            .values()
            .cloned()
            .collect();
        let leased = self.leased.lock().await;
        let dead_letters = self.dead_letters.lock().await;
        let mut stats = Vec::with_capacity(routes.len());

        for topology in routes {
            let queue = transport
                .channel
                .queue_declare(
                    &topology.queue_name,
                    QueueDeclareOptions {
                        passive: true,
                        ..QueueDeclareOptions::default()
                    },
                    FieldTable::default(),
                )
                .await
                .map_err(|error| {
                    rabbitmq_error(format!(
                        "failed to inspect rabbitmq queue {}: {error}",
                        topology.queue_name
                    ))
                })?;

            let leased_count = leased
                .values()
                .filter(|item| {
                    item.delivery.task.queue == topology.queue
                        && item.delivery.task.lane == topology.lane
                })
                .count();
            let dead_letter_count = dead_letters
                .iter()
                .filter(|item| item.queue == topology.queue && item.lane == topology.lane)
                .count();

            stats.push(RuntimeQueueStats {
                queue: topology.queue,
                lane: topology.lane,
                queued: queue.message_count() as usize,
                leased: leased_count,
                dead_lettered: dead_letter_count,
            });
        }

        Ok(stats)
    }

    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        if self.transport.is_none() {
            return self.inner.dead_letters().await;
        }

        Ok(self.dead_letters.lock().await.clone())
    }

    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        if self.transport.is_none() {
            return self.inner.replay_dead_letter(delivery_id).await;
        }

        let mut dead_letters = self.dead_letters.lock().await;
        let Some(position) = dead_letters
            .iter()
            .position(|record| record.delivery_id == delivery_id)
        else {
            return Err(AppError::NotFound(format!(
                "dead letter not found: {delivery_id}"
            )));
        };
        let record = dead_letters.remove(position);
        drop(dead_letters);

        let topology = self.ensure_route_topology(&record.task)?;
        self.publish_envelope(
            &topology,
            &topology.exchange,
            &topology.routing_key,
            &RabbitMqEnvelope {
                task: record.task.clone(),
                attempt: 1,
                last_error: Some(format!("replayed from dead letter: {}", record.error)),
            },
            None,
        )
        .await?;

        Ok(RuntimeQueueReceipt {
            task_id: record.task.task_id,
            queue: record.task.queue,
            lane: record.task.lane,
            status: crate::RuntimeTaskLifecycleStatus::Queued,
        })
    }
}

fn validate_rabbitmq_config(config: &RabbitMqQueueConfig) -> AppResult<()> {
    if !(config.url.starts_with("amqp://") || config.url.starts_with("amqps://")) {
        return Err(AppError::InvalidConfig(
            "rabbitmq url must start with amqp:// or amqps://".to_owned(),
        ));
    }
    if config.exchange.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "rabbitmq exchange cannot be empty".to_owned(),
        ));
    }
    if config.queue_prefix.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "rabbitmq queue_prefix cannot be empty".to_owned(),
        ));
    }
    Ok(())
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

fn rabbitmq_error(message: String) -> AppError {
    AppError::Database(message)
}

fn rabbitmq_connection_properties() -> ConnectionProperties {
    #[cfg(unix)]
    {
        ConnectionProperties::default()
            .with_executor(tokio_executor_trait::Tokio::current())
            .with_reactor(tokio_reactor_trait::Tokio)
    }

    #[cfg(not(unix))]
    {
        ConnectionProperties::default().with_executor(tokio_executor_trait::Tokio::current())
    }
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
