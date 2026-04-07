use super::*;
use crate::{
    observe_broker_dead_letter, observe_broker_operation, observe_broker_operation_failure,
    observe_broker_retry,
};

pub(super) struct RabbitMqDeadLetterReplay {
    target: RabbitMqDeadLetterMessage,
    remaining: Vec<RabbitMqDeadLetterMessage>,
}

impl RabbitMqBrokerAdapter {
    #[cfg(test)]
    pub fn new(config: RabbitMqQueueConfig) -> AppResult<Self> {
        validate_rabbitmq_config(&config)?;
        Ok(Self {
            config,
            broker_enabled: false,
            inner: InMemoryRuntimeTaskQueue::default(),
            transport: AsyncMutex::new(None),
            routes: RouteCatalog::new(),
            leased: AsyncMutex::new(HashMap::new()),
        })
    }

    pub async fn connect(config: RabbitMqQueueConfig) -> AppResult<Self> {
        validate_rabbitmq_config(&config)?;
        let transport = Self::connect_transport(&config).await?;

        Ok(Self {
            config,
            broker_enabled: true,
            inner: InMemoryRuntimeTaskQueue::default(),
            transport: AsyncMutex::new(Some(Arc::new(transport))),
            routes: RouteCatalog::new(),
            leased: AsyncMutex::new(HashMap::new()),
        })
    }

    async fn connect_transport(config: &RabbitMqQueueConfig) -> AppResult<RabbitMqTransport> {
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

        Ok(RabbitMqTransport {
            connection,
            channel,
        })
    }

    async fn active_transport(&self) -> AppResult<Option<Arc<RabbitMqTransport>>> {
        if !self.broker_enabled {
            return Ok(None);
        }

        let mut guard = self.transport.lock().await;
        let needs_reconnect = guard
            .as_ref()
            .map(|transport| !transport.is_connected())
            .unwrap_or(true);
        if !needs_reconnect {
            return Ok(guard.clone());
        }

        let transport = Arc::new(Self::connect_transport(&self.config).await?);
        *guard = Some(transport.clone());
        drop(guard);
        self.leased.lock().await.clear();
        info!(
            broker = "rabbitmq",
            exchange = %self.config.exchange,
            queue_prefix = %self.config.queue_prefix,
            "reconnected rabbitmq transport"
        );
        Ok(Some(transport))
    }

    async fn invalidate_transport(&self) {
        if !self.broker_enabled {
            return;
        }
        *self.transport.lock().await = None;
        self.leased.lock().await.clear();
        info!(
            broker = "rabbitmq",
            exchange = %self.config.exchange,
            queue_prefix = %self.config.queue_prefix,
            "invalidated rabbitmq transport after broker operation failure"
        );
    }

    async fn with_transport_result<T>(&self, result: AppResult<T>) -> AppResult<T> {
        if result.is_err() {
            self.invalidate_transport().await;
        }
        result
    }

    #[cfg(test)]
    pub fn declared_topologies(&self) -> AppResult<Vec<RabbitMqRouteTopology>> {
        self.routes.topologies()
    }

    fn ensure_route_topology(&self, task: &RuntimeTask) -> AppResult<RabbitMqRouteTopology> {
        self.routes.get_or_insert_with(&task.queue, &task.lane, || {
            build_route_topology(&self.config, &task.queue, &task.lane)
        })
    }

    fn topology_for_route(
        &self,
        route: &(String, String),
    ) -> AppResult<Option<RabbitMqRouteTopology>> {
        self.routes.get(route)
    }

    fn rotate_route(&self, route: &(String, String)) {
        self.routes.rotate(route);
    }

    async fn publish_envelope(
        &self,
        topology: &RabbitMqRouteTopology,
        exchange: &str,
        routing_key: &str,
        envelope: &RabbitMqEnvelope,
        expiration_ms: Option<u64>,
    ) -> AppResult<()> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(());
        };

        self.with_transport_result(transport.ensure_topology(topology).await)
            .await?;

        let payload = serde_json::to_vec(envelope).map_err(|_| AppError::Internal)?;
        let mut properties = BasicProperties::default()
            .with_content_type("application/json".into())
            .with_delivery_mode(2);
        if let Some(expiration_ms) = expiration_ms {
            properties = properties.with_expiration(expiration_ms.to_string().into());
        }

        let confirm = self
            .with_transport_result(
                transport
                    .channel
                    .basic_publish(
                        exchange,
                        routing_key,
                        BasicPublishOptions::default(),
                        &payload,
                        properties,
                    )
                    .await
                    .map_err(|error| {
                        rabbitmq_error(format!("failed to publish runtime task: {error}"))
                    }),
            )
            .await?;
        self.with_transport_result(confirm.await.map_err(|error| {
            rabbitmq_error(format!(
                "rabbitmq did not confirm runtime task publish: {error}"
            ))
        }))
        .await?;
        Ok(())
    }

    async fn reserve_from_broker(
        &self,
        bindings: &[RuntimeRouteBinding],
    ) -> AppResult<Option<RuntimeTaskDelivery>> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(None);
        };
        let routes_snapshot = self.routes.ordered_routes(bindings)?;

        for route in routes_snapshot {
            let Some(topology) = self.topology_for_route(&route)? else {
                continue;
            };

            let delivery = self
                .with_transport_result(
                    transport
                        .channel
                        .basic_get(&topology.queue_name, BasicGetOptions { no_ack: false })
                        .await
                        .map_err(|error| {
                            rabbitmq_error(format!(
                                "failed to reserve runtime task from {}: {error}",
                                topology.queue_name
                            ))
                        }),
                )
                .await?;

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
            debug!(
                broker = "rabbitmq",
                task_id = %runtime_delivery.task.task_id,
                queue = %runtime_delivery.task.queue,
                lane = %runtime_delivery.task.lane,
                attempt = runtime_delivery.attempt,
                delivery_id = %runtime_delivery.delivery_id,
                "reserved runtime task from rabbitmq"
            );

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
        let Some(transport) = self.active_transport().await? else {
            return Ok(());
        };
        let leased = self.leased.lock().await.remove(delivery_id);
        if let Some(leased) = leased {
            self.with_transport_result(
                transport
                    .channel
                    .basic_ack(leased.delivery_tag, BasicAckOptions::default())
                    .await
                    .map_err(|error| {
                        rabbitmq_error(format!(
                            "failed to ack rabbitmq delivery {}: {error}",
                            leased.delivery_tag
                        ))
                    }),
            )
            .await?;
        }
        Ok(())
    }

    async fn push_dead_letter_record(
        &self,
        delivery: &RuntimeTaskDelivery,
        error: &str,
    ) -> RuntimeDeadLetterRecord {
        build_dead_letter_record(delivery, error)
    }

    async fn publish_dead_letter_record(
        &self,
        topology: &RabbitMqRouteTopology,
        record: &RuntimeDeadLetterRecord,
    ) -> AppResult<()> {
        let payload = serde_json::to_vec(record).map_err(|_| AppError::Internal)?;
        self.publish_raw_payload(
            topology,
            &topology.dead_letter_exchange,
            &topology.dead_letter_routing_key,
            &payload,
            None,
        )
        .await
    }

    async fn publish_raw_payload(
        &self,
        topology: &RabbitMqRouteTopology,
        exchange: &str,
        routing_key: &str,
        payload: &[u8],
        expiration_ms: Option<u64>,
    ) -> AppResult<()> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(());
        };

        self.with_transport_result(transport.ensure_topology(topology).await)
            .await?;

        let mut properties = BasicProperties::default()
            .with_content_type("application/json".into())
            .with_delivery_mode(2);
        if let Some(expiration_ms) = expiration_ms {
            properties = properties.with_expiration(expiration_ms.to_string().into());
        }

        let confirm = self
            .with_transport_result(
                transport
                    .channel
                    .basic_publish(
                        exchange,
                        routing_key,
                        BasicPublishOptions::default(),
                        payload,
                        properties,
                    )
                    .await
                    .map_err(|error| {
                        rabbitmq_error(format!("failed to publish runtime task: {error}"))
                    }),
            )
            .await?;
        self.with_transport_result(confirm.await.map_err(|error| {
            rabbitmq_error(format!(
                "rabbitmq did not confirm runtime task publish: {error}"
            ))
        }))
        .await?;
        Ok(())
    }

    async fn drain_dead_letter_queue(
        &self,
        topology: &RabbitMqRouteTopology,
    ) -> AppResult<Vec<RabbitMqDeadLetterMessage>> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(Vec::new());
        };
        self.with_transport_result(transport.ensure_topology(topology).await)
            .await?;

        let mut messages = Vec::new();
        loop {
            let delivery = self
                .with_transport_result(
                    transport
                        .channel
                        .basic_get(
                            &topology.dead_letter_queue_name,
                            BasicGetOptions { no_ack: false },
                        )
                        .await
                        .map_err(|error| {
                            rabbitmq_error(format!(
                                "failed to reserve dead-letter record from {}: {error}",
                                topology.dead_letter_queue_name
                            ))
                        }),
                )
                .await?;

            let Some(delivery) = delivery else {
                break;
            };
            let record: RuntimeDeadLetterRecord =
                serde_json::from_slice(&delivery.data).map_err(|_| AppError::Internal)?;
            messages.push(RabbitMqDeadLetterMessage {
                delivery_tag: delivery.delivery_tag,
                record,
                payload: delivery.data.clone(),
            });
        }

        Ok(messages)
    }

    async fn restore_dead_letter_queue(
        &self,
        topology: &RabbitMqRouteTopology,
        messages: Vec<RabbitMqDeadLetterMessage>,
    ) -> AppResult<()> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(());
        };

        for message in messages {
            self.publish_raw_payload(
                topology,
                &topology.dead_letter_exchange,
                &topology.dead_letter_routing_key,
                &message.payload,
                None,
            )
            .await?;
            self.with_transport_result(
                transport
                    .channel
                    .basic_ack(message.delivery_tag, BasicAckOptions::default())
                    .await
                    .map_err(|error| {
                        rabbitmq_error(format!(
                            "failed to ack rabbitmq dead-letter delivery {}: {error}",
                            message.delivery_tag
                        ))
                    }),
            )
            .await?;
        }

        Ok(())
    }

    async fn load_dead_letters_from_broker(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        self.ensure_registered_routes_loaded().await?;
        let routes: Vec<_> = self.routes.topologies()?;
        let mut records = Vec::new();

        for topology in routes {
            let messages = self.drain_dead_letter_queue(&topology).await?;
            records.extend(messages.iter().map(|message| message.record.clone()));
            self.restore_dead_letter_queue(&topology, messages).await?;
        }

        Ok(records)
    }

    async fn find_dead_letter_replay(
        &self,
        delivery_id: &str,
    ) -> AppResult<Option<DeadLetterReplay<RabbitMqRouteTopology, RabbitMqDeadLetterReplay>>> {
        self.ensure_registered_routes_loaded().await?;

        for topology in self.routes.topologies()? {
            let drained = self.drain_dead_letter_queue(&topology).await?;
            if drained.is_empty() {
                continue;
            }

            let mut target = None;
            let mut remaining = Vec::new();
            for message in drained {
                if message.record.delivery_id == delivery_id && target.is_none() {
                    target = Some(message);
                } else {
                    remaining.push(message);
                }
            }

            let Some(target) = target else {
                self.restore_dead_letter_queue(&topology, remaining).await?;
                continue;
            };

            let record = target.record.clone();
            return Ok(Some(DeadLetterReplay {
                topology,
                record,
                stored: RabbitMqDeadLetterReplay { target, remaining },
            }));
        }

        Ok(None)
    }

    async fn finalize_replayed_dead_letter(
        &self,
        topology: &RabbitMqRouteTopology,
        replay: RabbitMqDeadLetterReplay,
    ) -> AppResult<()> {
        let Some(transport) = self.active_transport().await? else {
            return Err(AppError::Internal);
        };
        self.with_transport_result(
            transport
                .channel
                .basic_ack(replay.target.delivery_tag, BasicAckOptions::default())
                .await
                .map_err(|error| {
                    rabbitmq_error(format!(
                        "failed to ack replayed rabbitmq dead-letter delivery {}: {error}",
                        replay.target.delivery_tag
                    ))
                }),
        )
        .await?;
        self.restore_dead_letter_queue(topology, replay.remaining)
            .await
    }

    async fn persist_route_registry(&self, topology: &RabbitMqRouteTopology) -> AppResult<()> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(());
        };
        let queue_name = rabbitmq_route_registry_queue_name(&self.config);
        self.with_transport_result(
            transport
                .channel
                .queue_declare(
                    &queue_name,
                    QueueDeclareOptions {
                        durable: true,
                        ..QueueDeclareOptions::default()
                    },
                    FieldTable::default(),
                )
                .await
                .map_err(|error| {
                    rabbitmq_error(format!(
                        "failed to declare rabbitmq route registry queue {}: {error}",
                        queue_name
                    ))
                }),
        )
        .await?;

        let payload = serde_json::to_vec(&RabbitMqRouteRegistryEntry {
            queue: topology.queue.clone(),
            lane: topology.lane.clone(),
        })
        .map_err(|_| AppError::Internal)?;
        let confirm = self
            .with_transport_result(
                transport
                    .channel
                    .basic_publish(
                        "",
                        &queue_name,
                        BasicPublishOptions::default(),
                        &payload,
                        BasicProperties::default()
                            .with_content_type("application/json".into())
                            .with_delivery_mode(2),
                    )
                    .await
                    .map_err(|error| {
                        rabbitmq_error(format!(
                            "failed to publish rabbitmq route registry entry: {error}"
                        ))
                    }),
            )
            .await?;
        self.with_transport_result(confirm.await.map_err(|error| {
            rabbitmq_error(format!(
                "rabbitmq did not confirm route registry publish: {error}"
            ))
        }))
        .await?;
        Ok(())
    }

    async fn ensure_registered_routes_loaded(&self) -> AppResult<()> {
        let Some(transport) = self.active_transport().await? else {
            return Ok(());
        };
        let queue_name = rabbitmq_route_registry_queue_name(&self.config);
        self.with_transport_result(
            transport
                .channel
                .queue_declare(
                    &queue_name,
                    QueueDeclareOptions {
                        durable: true,
                        ..QueueDeclareOptions::default()
                    },
                    FieldTable::default(),
                )
                .await
                .map_err(|error| {
                    rabbitmq_error(format!(
                        "failed to declare rabbitmq route registry queue {}: {error}",
                        queue_name
                    ))
                }),
        )
        .await?;

        let mut entries = Vec::new();
        loop {
            let delivery = self
                .with_transport_result(
                    transport
                        .channel
                        .basic_get(&queue_name, BasicGetOptions { no_ack: false })
                        .await
                        .map_err(|error| {
                            rabbitmq_error(format!(
                                "failed to read rabbitmq route registry queue {}: {error}",
                                queue_name
                            ))
                        }),
                )
                .await?;
            let Some(delivery) = delivery else {
                break;
            };
            let payload = delivery.data.clone();
            let entry: RabbitMqRouteRegistryEntry =
                serde_json::from_slice(&payload).map_err(|_| AppError::Internal)?;
            entries.push((delivery.delivery_tag, payload, entry));
        }

        let mut seen = std::collections::BTreeSet::new();
        for (_, _, entry) in &entries {
            if !seen.insert(entry.clone()) {
                continue;
            }
            let topology = build_route_topology(&self.config, &entry.queue, &entry.lane);
            self.with_transport_result(transport.ensure_topology(&topology).await)
                .await?;
            self.routes
                .register_route(&entry.queue, &entry.lane, topology)?;
        }

        for (delivery_tag, payload, _) in entries {
            let confirm = self
                .with_transport_result(
                    transport
                        .channel
                        .basic_publish(
                            "",
                            &queue_name,
                            BasicPublishOptions::default(),
                            &payload,
                            BasicProperties::default()
                                .with_content_type("application/json".into())
                                .with_delivery_mode(2),
                        )
                        .await
                        .map_err(|error| {
                            rabbitmq_error(format!(
                                "failed to restore rabbitmq route registry entry: {error}"
                            ))
                        }),
                )
                .await?;
            self.with_transport_result(confirm.await.map_err(|error| {
                rabbitmq_error(format!(
                    "rabbitmq did not confirm route registry restore publish: {error}"
                ))
            }))
            .await?;
            self.with_transport_result(
                transport
                    .channel
                    .basic_ack(delivery_tag, BasicAckOptions::default())
                    .await
                    .map_err(|error| {
                        rabbitmq_error(format!(
                            "failed to ack rabbitmq route registry entry {}: {error}",
                            delivery_tag
                        ))
                    }),
            )
            .await?;
        }

        Ok(())
    }
}

#[async_trait]
impl BrokerDeadLetterStore for RabbitMqBrokerAdapter {
    type Topology = RabbitMqRouteTopology;
    type Stored = RabbitMqDeadLetterReplay;

    async fn store_dead_letter(
        &self,
        topology: &Self::Topology,
        record: &RuntimeDeadLetterRecord,
    ) -> AppResult<()> {
        self.publish_dead_letter_record(topology, record).await
    }

    async fn load_dead_letter_records(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        self.load_dead_letters_from_broker().await
    }

    async fn find_dead_letter_for_replay(
        &self,
        delivery_id: &str,
    ) -> AppResult<Option<DeadLetterReplay<Self::Topology, Self::Stored>>> {
        self.find_dead_letter_replay(delivery_id).await
    }

    async fn remove_replayed_dead_letter(
        &self,
        replay: DeadLetterReplay<Self::Topology, Self::Stored>,
    ) -> AppResult<()> {
        self.finalize_replayed_dead_letter(&replay.topology, replay.stored)
            .await
    }
}

impl RabbitMqTransport {
    fn is_connected(&self) -> bool {
        self.connection.status().connected() && self.channel.status().connected()
    }

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
impl BrokerAdapter for RabbitMqBrokerAdapter {
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()> {
        let topology = self.ensure_route_topology(&task)?;
        let queue = task.queue.clone();
        let lane = task.lane.clone();
        if !self.broker_enabled {
            return self.inner.enqueue(task).await;
        }
        self.persist_route_registry(&topology).await?;
        debug!(
            broker = "rabbitmq",
            task_id = %task.task_id,
            queue = %task.queue,
            lane = %task.lane,
            exchange = %topology.exchange,
            routing_key = %topology.routing_key,
            "publishing runtime task to rabbitmq"
        );

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
        .map(|_| {
            observe_broker_operation("rabbitmq", queue.as_str(), lane.as_str(), "enqueue");
        })
        .map_err(|error| {
            observe_broker_operation_failure("rabbitmq", queue.as_str(), lane.as_str(), "enqueue");
            error
        })
    }

    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery> {
        if !self.broker_enabled {
            return self.inner.reserve(bindings).await;
        }

        match self.reserve_from_broker(bindings).await {
            Ok(Some(delivery)) => {
                observe_broker_operation(
                    "rabbitmq",
                    delivery.task.queue.as_str(),
                    delivery.task.lane.as_str(),
                    "reserve",
                );
                Some(delivery)
            }
            Ok(None) => None,
            Err(_) => {
                observe_broker_operation_failure("rabbitmq", "*", "*", "reserve");
                None
            }
        }
    }

    async fn ack(&self, delivery_id: &str) -> AppResult<()> {
        if !self.broker_enabled {
            return self.inner.ack(delivery_id).await;
        }

        let leased = self.leased.lock().await.get(delivery_id).cloned();
        self.ack_broker_delivery(delivery_id)
            .await
            .map(|_| {
                if let Some(leased) = leased {
                    observe_broker_operation(
                        "rabbitmq",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "ack",
                    );
                }
            })
            .map_err(|error| {
                observe_broker_operation_failure("rabbitmq", "*", "*", "ack");
                error
            })
    }

    async fn retry(
        &self,
        delivery_id: &str,
        error: &str,
        delay_ms: u64,
    ) -> AppResult<RetryDisposition> {
        if !self.broker_enabled {
            return self.inner.retry(delivery_id, error, delay_ms).await;
        }

        let Some(leased) = self.leased.lock().await.remove(delivery_id) else {
            observe_broker_operation_failure("rabbitmq", "*", "*", "retry");
            return Err(AppError::Internal);
        };

        if leased.delivery.attempt >= leased.delivery.task.retry_policy.max_attempts {
            let record = self.push_dead_letter_record(&leased.delivery, error).await;
            observe_broker_retry(
                "rabbitmq",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "dead_lettered",
            );
            observe_broker_dead_letter(
                "rabbitmq",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "retry_exhausted",
            );
            self.store_dead_letter(&leased.topology, &record)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "rabbitmq",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
            self.ack_broker_delivery(&leased.delivery.delivery_id)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "rabbitmq",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
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
        .await
        .map_err(|error| {
            observe_broker_operation_failure(
                "rabbitmq",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "retry",
            );
            error
        })?;
        self.ack_broker_delivery(&leased.delivery.delivery_id)
            .await
            .map_err(|error| {
                observe_broker_operation_failure(
                    "rabbitmq",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "retry",
                );
                error
            })?;
        observe_broker_retry(
            "rabbitmq",
            leased.delivery.task.queue.as_str(),
            leased.delivery.task.lane.as_str(),
            "requeued",
        );
        Ok(RetryDisposition::Requeued)
    }

    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()> {
        if !self.broker_enabled {
            return self.inner.reject(delivery_id, error).await;
        }

        let Some(leased) = self.leased.lock().await.remove(delivery_id) else {
            return Ok(());
        };
        let record = self.push_dead_letter_record(&leased.delivery, error).await;
        observe_broker_dead_letter(
            "rabbitmq",
            leased.delivery.task.queue.as_str(),
            leased.delivery.task.lane.as_str(),
            "rejected",
        );
        self.store_dead_letter(&leased.topology, &record)
            .await
            .map_err(|error| {
                observe_broker_operation_failure(
                    "rabbitmq",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "reject",
                );
                error
            })?;
        self.ack_broker_delivery(&leased.delivery.delivery_id)
            .await
            .map_err(|error| {
                observe_broker_operation_failure(
                    "rabbitmq",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "reject",
                );
                error
            })
    }

    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        if !self.broker_enabled {
            return self.inner.stats().await;
        }
        let Some(transport) = self.active_transport().await? else {
            return Ok(Vec::new());
        };
        self.ensure_registered_routes_loaded().await?;

        let routes: Vec<_> = self.routes.topologies()?;
        let leased = self.leased.lock().await;
        let mut stats = Vec::with_capacity(routes.len());

        for topology in routes {
            let queue = self
                .with_transport_result(
                    transport
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
                        }),
                )
                .await?;
            let dead_letter_queue = self
                .with_transport_result(
                    transport
                        .channel
                        .queue_declare(
                            &topology.dead_letter_queue_name,
                            QueueDeclareOptions {
                                passive: true,
                                ..QueueDeclareOptions::default()
                            },
                            FieldTable::default(),
                        )
                        .await
                        .map_err(|error| {
                            rabbitmq_error(format!(
                                "failed to inspect rabbitmq dead-letter queue {}: {error}",
                                topology.dead_letter_queue_name
                            ))
                        }),
                )
                .await?;

            let leased_count = leased
                .values()
                .filter(|item| {
                    item.delivery.task.queue == topology.queue
                        && item.delivery.task.lane == topology.lane
                })
                .count();

            stats.push(RuntimeQueueStats {
                queue: topology.queue,
                lane: topology.lane,
                queued: queue.message_count() as usize,
                leased: leased_count,
                dead_lettered: dead_letter_queue.message_count() as usize,
            });
        }

        Ok(stats)
    }

    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        if !self.broker_enabled {
            return self.inner.dead_letters().await;
        }

        self.load_dead_letter_records().await
    }

    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        if !self.broker_enabled {
            return self.inner.replay_dead_letter(delivery_id).await;
        }
        replay_dead_letter_with_store(
            "rabbitmq",
            self,
            delivery_id,
            |topology, record| async move {
                self.publish_envelope(
                    &topology,
                    &topology.exchange,
                    &topology.routing_key,
                    &build_rabbitmq_replay_envelope(&record),
                    None,
                )
                .await
            },
        )
        .await
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
