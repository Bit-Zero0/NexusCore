use super::*;
use crate::{
    observe_broker_dead_letter, observe_broker_operation, observe_broker_operation_failure,
    observe_broker_retry,
};

pub(super) struct NatsDeadLetterReplay {
    sequence: u64,
}

impl NatsBrokerAdapter {
    pub async fn connect(config: NatsQueueConfig) -> AppResult<Self> {
        validate_nats_config(&config)?;
        let ack_wait_ms = config.ack_wait_ms;
        let client = async_nats::connect(config.url.clone())
            .await
            .map_err(|error| nats_error(format!("failed to connect nats: {error}")))?;
        let jetstream = jetstream::new(client.clone());
        let work_stream = jetstream
            .get_or_create_stream(stream::Config {
                name: config.stream_name.clone(),
                subjects: vec![format!("{}.>", config.subject_prefix)],
                retention: RetentionPolicy::WorkQueue,
                storage: StorageType::File,
                ..Default::default()
            })
            .await
            .map_err(|error| nats_error(format!("failed to create nats stream: {error}")))?;
        let dead_letter_stream_name = nats_dead_letter_stream_name(&config);
        let dead_letter_stream = jetstream
            .get_or_create_stream(stream::Config {
                name: dead_letter_stream_name.clone(),
                subjects: vec![format!("{}.>", nats_dead_letter_subject_prefix(&config))],
                retention: RetentionPolicy::Limits,
                storage: StorageType::File,
                allow_direct: true,
                ..Default::default()
            })
            .await
            .map_err(|error| {
                nats_error(format!(
                    "failed to create nats dead-letter stream {dead_letter_stream_name}: {error}"
                ))
            })?;

        Ok(Self {
            config,
            transport: NatsTransport {
                _client: client,
                jetstream,
                work_stream,
                dead_letter_stream,
                ack_wait_ms,
            },
            routes: RouteCatalog::new(),
            leased: AsyncMutex::new(HashMap::new()),
        })
    }

    fn ensure_route_topology(&self, task: &RuntimeTask) -> AppResult<NatsRouteTopology> {
        self.routes.get_or_insert_with(&task.queue, &task.lane, || {
            build_nats_route_topology(&self.config, &task.queue, &task.lane)
        })
    }

    fn topology_for_route(&self, route: &(String, String)) -> AppResult<Option<NatsRouteTopology>> {
        self.routes.get(route)
    }

    fn rotate_route(&self, route: &(String, String)) {
        self.routes.rotate(route);
    }

    async fn publish_envelope(
        &self,
        topology: &NatsRouteTopology,
        envelope: &NatsEnvelope,
    ) -> AppResult<()> {
        self.transport.ensure_topology(topology).await?;
        let payload = serde_json::to_vec(envelope).map_err(|_| AppError::Internal)?;
        self.transport
            .jetstream
            .publish(topology.subject.clone(), payload.into())
            .await
            .map_err(|error| {
                nats_error(format!("failed to publish runtime task to nats: {error}"))
            })?
            .await
            .map_err(|error| {
                nats_error(format!(
                    "nats did not confirm runtime task publish: {error}"
                ))
            })?;
        Ok(())
    }

    async fn publish_dead_letter_record(
        &self,
        topology: &NatsRouteTopology,
        record: &RuntimeDeadLetterRecord,
    ) -> AppResult<()> {
        let payload = serde_json::to_vec(record).map_err(|_| AppError::Internal)?;
        self.transport
            .jetstream
            .publish(topology.dead_letter_subject.clone(), payload.into())
            .await
            .map_err(|error| {
                nats_error(format!(
                    "failed to publish dead-letter record to nats: {error}"
                ))
            })?
            .await
            .map_err(|error| {
                nats_error(format!(
                    "nats did not confirm dead-letter record publish: {error}"
                ))
            })?;
        Ok(())
    }

    async fn load_dead_letters(&self) -> AppResult<Vec<(u64, RuntimeDeadLetterRecord)>> {
        let stream_info = self
            .transport
            .dead_letter_stream
            .get_info()
            .await
            .map_err(|error| {
                nats_error(format!(
                    "failed to inspect nats dead-letter stream: {error}"
                ))
            })?;

        if stream_info.state.messages == 0 {
            return Ok(Vec::new());
        }

        let mut records = Vec::with_capacity(stream_info.state.messages as usize);
        for sequence in stream_info.state.first_sequence..=stream_info.state.last_sequence {
            let message = match self
                .transport
                .dead_letter_stream
                .get_raw_message(sequence)
                .await
            {
                Ok(message) => message,
                Err(error) => {
                    if matches!(error.kind(), stream::RawMessageErrorKind::NoMessageFound) {
                        continue;
                    }
                    return Err(nats_error(format!(
                        "failed to load nats dead-letter message {sequence}: {error}"
                    )));
                }
            };
            let record: RuntimeDeadLetterRecord =
                serde_json::from_slice(message.payload.as_ref()).map_err(|_| AppError::Internal)?;
            records.push((sequence, record));
        }

        Ok(records)
    }

    async fn find_dead_letter(
        &self,
        delivery_id: &str,
    ) -> AppResult<Option<(u64, RuntimeDeadLetterRecord)>> {
        let records = self.load_dead_letters().await?;
        Ok(records
            .into_iter()
            .find(|(_, record)| record.delivery_id == delivery_id))
    }

    async fn find_dead_letter_replay(
        &self,
        delivery_id: &str,
    ) -> AppResult<Option<DeadLetterReplay<NatsRouteTopology, NatsDeadLetterReplay>>> {
        let Some((sequence, record)) = self.find_dead_letter(delivery_id).await? else {
            return Ok(None);
        };
        let topology = self.ensure_route_topology(&record.task)?;
        Ok(Some(DeadLetterReplay {
            topology,
            record,
            stored: NatsDeadLetterReplay { sequence },
        }))
    }

    async fn remove_replayed_dead_letter_sequence(&self, sequence: u64) -> AppResult<()> {
        self.transport
            .dead_letter_stream
            .delete_message(sequence)
            .await
            .map_err(|error| {
                nats_error(format!(
                    "failed to delete replayed nats dead-letter message {sequence}: {error}"
                ))
            })?;
        Ok(())
    }

    async fn reserve_from_broker(
        &self,
        bindings: &[RuntimeRouteBinding],
    ) -> AppResult<Option<RuntimeTaskDelivery>> {
        let routes_snapshot = self.routes.ordered_routes(bindings)?;

        for route in routes_snapshot {
            let Some(topology) = self.topology_for_route(&route)? else {
                continue;
            };
            self.transport.ensure_topology(&topology).await?;
            let consumer: consumer::PullConsumer = self.transport.consumer(&topology).await?;
            let mut messages = consumer
                .fetch()
                .max_messages(1)
                .expires(Duration::from_millis(200))
                .messages()
                .await
                .map_err(|error| {
                    nats_error(format!("failed to fetch runtime task from nats: {error}"))
                })?;
            let Some(message) = messages.next().await else {
                continue;
            };
            let message = message
                .map_err(|error| nats_error(format!("failed to receive nats message: {error}")))?;
            self.rotate_route(&route);

            let metadata = message.info().map_err(|error| {
                nats_error(format!("failed to decode nats message metadata: {error}"))
            })?;
            let delivery_sequence = metadata.stream_sequence;
            let attempt = metadata.delivered as u32;
            let (message, acker) = message.split();
            let envelope: NatsEnvelope =
                serde_json::from_slice(message.payload.as_ref()).map_err(|_| AppError::Internal)?;
            let delivery_id = format!("nats-{}-{delivery_sequence}", envelope.task.task_id);
            let runtime_delivery = RuntimeTaskDelivery {
                delivery_id: delivery_id.clone(),
                attempt,
                leased_until: Some(now_ms() + self.config.ack_wait_ms),
                last_error: envelope.last_error.clone(),
                task: envelope.task,
            };

            if attempt > 1 {
                observe_broker_reclaim(
                    "nats",
                    runtime_delivery.task.queue.as_str(),
                    runtime_delivery.task.lane.as_str(),
                );
                info!(
                    broker = "nats",
                    task_id = %runtime_delivery.task.task_id,
                    queue = %runtime_delivery.task.queue,
                    lane = %runtime_delivery.task.lane,
                    attempt = runtime_delivery.attempt,
                    delivery_id = %runtime_delivery.delivery_id,
                    lease_recovered = true,
                    ack_wait_ms = self.config.ack_wait_ms,
                    "reclaimed runtime task lease from nats after redelivery"
                );
            } else {
                debug!(
                    broker = "nats",
                    task_id = %runtime_delivery.task.task_id,
                    queue = %runtime_delivery.task.queue,
                    lane = %runtime_delivery.task.lane,
                    attempt = runtime_delivery.attempt,
                    delivery_id = %runtime_delivery.delivery_id,
                    leased_until_ms = runtime_delivery.leased_until,
                    "reserved runtime task from nats"
                );
            }

            self.leased.lock().await.insert(
                delivery_id,
                NatsLeasedDelivery {
                    acker,
                    delivery: runtime_delivery.clone(),
                },
            );
            return Ok(Some(runtime_delivery));
        }

        Ok(None)
    }
}

#[async_trait]
impl BrokerDeadLetterStore for NatsBrokerAdapter {
    type Topology = NatsRouteTopology;
    type Stored = NatsDeadLetterReplay;

    async fn store_dead_letter(
        &self,
        topology: &Self::Topology,
        record: &RuntimeDeadLetterRecord,
    ) -> AppResult<()> {
        self.publish_dead_letter_record(topology, record).await
    }

    async fn load_dead_letter_records(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        Ok(self
            .load_dead_letters()
            .await?
            .into_iter()
            .map(|(_, record)| record)
            .collect())
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
        self.remove_replayed_dead_letter_sequence(replay.stored.sequence)
            .await
    }
}

impl NatsTransport {
    async fn ensure_topology(&self, topology: &NatsRouteTopology) -> AppResult<()> {
        self.consumer(topology).await?;
        Ok(())
    }

    async fn consumer(&self, topology: &NatsRouteTopology) -> AppResult<consumer::PullConsumer> {
        self.work_stream
            .get_or_create_consumer::<pull::Config>(
                &topology.consumer_name,
                pull::Config {
                    durable_name: Some(topology.consumer_name.clone()),
                    ack_policy: consumer::AckPolicy::Explicit,
                    ack_wait: Duration::from_millis(self.ack_wait_ms),
                    filter_subject: topology.subject.clone(),
                    inactive_threshold: Duration::from_secs(60 * 60),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| {
                nats_error(format!(
                    "failed to create nats consumer {}: {error}",
                    topology.consumer_name
                ))
            })
    }
}

#[async_trait]
impl BrokerAdapter for NatsBrokerAdapter {
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()> {
        let topology = self.ensure_route_topology(&task)?;
        let queue = task.queue.clone();
        let lane = task.lane.clone();
        debug!(
            broker = "nats",
            task_id = %task.task_id,
            queue = %task.queue,
            lane = %task.lane,
            subject = %topology.subject,
            "publishing runtime task to nats"
        );
        self.publish_envelope(
            &topology,
            &NatsEnvelope {
                task,
                last_error: None,
            },
        )
        .await
        .map(|_| {
            observe_broker_operation("nats", queue.as_str(), lane.as_str(), "enqueue");
        })
        .map_err(|error| {
            observe_broker_operation_failure("nats", queue.as_str(), lane.as_str(), "enqueue");
            error
        })
    }

    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery> {
        match self.reserve_from_broker(bindings).await {
            Ok(Some(delivery)) => {
                observe_broker_operation(
                    "nats",
                    delivery.task.queue.as_str(),
                    delivery.task.lane.as_str(),
                    "reserve",
                );
                Some(delivery)
            }
            Ok(None) => None,
            Err(_) => {
                observe_broker_operation_failure("nats", "*", "*", "reserve");
                None
            }
        }
    }

    async fn ack(&self, delivery_id: &str) -> AppResult<()> {
        let leased = self.leased.lock().await.remove(delivery_id);
        if let Some(leased) = leased {
            let queue = leased.delivery.task.queue.clone();
            let lane = leased.delivery.task.lane.clone();
            leased
                .acker
                .ack()
                .await
                .map(|_| {
                    observe_broker_operation("nats", queue.as_str(), lane.as_str(), "ack");
                })
                .map_err(|error| {
                    observe_broker_operation_failure("nats", queue.as_str(), lane.as_str(), "ack");
                    nats_error(format!(
                        "failed to ack nats delivery {delivery_id}: {error}"
                    ))
                })?;
        }
        Ok(())
    }

    async fn retry(
        &self,
        delivery_id: &str,
        error: &str,
        delay_ms: u64,
    ) -> AppResult<RetryDisposition> {
        let Some(leased) = self.leased.lock().await.remove(delivery_id) else {
            observe_broker_operation_failure("nats", "*", "*", "retry");
            return Err(AppError::Internal);
        };

        if leased.delivery.attempt >= leased.delivery.task.retry_policy.max_attempts {
            let record = build_dead_letter_record(&leased.delivery, error);
            let topology = self.ensure_route_topology(&leased.delivery.task)?;
            observe_broker_retry(
                "nats",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "dead_lettered",
            );
            observe_broker_dead_letter(
                "nats",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "retry_exhausted",
            );
            self.store_dead_letter(&topology, &record)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "nats",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
            leased.acker.ack().await.map_err(|error| {
                observe_broker_operation_failure(
                    "nats",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "retry",
                );
                nats_error(format!(
                    "failed to ack dead-lettered nats delivery {delivery_id}: {error}"
                ))
            })?;
            return Ok(RetryDisposition::DeadLettered);
        }

        leased
            .acker
            .ack_with(AckKind::Nak(Some(Duration::from_millis(delay_ms))))
            .await
            .map_err(|error| {
                observe_broker_operation_failure(
                    "nats",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "retry",
                );
                nats_error(format!(
                    "failed to retry nats delivery {delivery_id}: {error}"
                ))
            })?;
        observe_broker_retry(
            "nats",
            leased.delivery.task.queue.as_str(),
            leased.delivery.task.lane.as_str(),
            "requeued",
        );
        Ok(RetryDisposition::Requeued)
    }

    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()> {
        let leased = self.leased.lock().await.remove(delivery_id);
        if let Some(leased) = leased {
            let record = build_dead_letter_record(&leased.delivery, error);
            let topology = self.ensure_route_topology(&leased.delivery.task)?;
            observe_broker_dead_letter(
                "nats",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "rejected",
            );
            self.store_dead_letter(&topology, &record)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "nats",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "reject",
                    );
                    error
                })?;
            leased.acker.ack().await.map_err(|error| {
                observe_broker_operation_failure(
                    "nats",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "reject",
                );
                nats_error(format!(
                    "failed to reject nats delivery {delivery_id}: {error}"
                ))
            })?;
        }
        Ok(())
    }

    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        let routes: Vec<_> = self.routes.topologies()?;
        let leased = self.leased.lock().await;
        let dead_letters = self.load_dead_letters().await?;
        let mut stats = Vec::with_capacity(routes.len());

        for topology in routes {
            let mut consumer: consumer::PullConsumer = self.transport.consumer(&topology).await?;
            let info = consumer.info().await.map_err(|error| {
                nats_error(format!(
                    "failed to inspect nats consumer {}: {error}",
                    topology.consumer_name
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
                .filter(|(_, item)| item.queue == topology.queue && item.lane == topology.lane)
                .count();

            stats.push(RuntimeQueueStats {
                queue: topology.queue,
                lane: topology.lane,
                queued: info.num_pending as usize,
                leased: leased_count,
                dead_lettered: dead_letter_count,
            });
        }

        Ok(stats)
    }

    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        self.load_dead_letter_records().await
    }

    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        replay_dead_letter_with_store("nats", self, delivery_id, |topology, record| async move {
            self.publish_envelope(&topology, &build_nats_replay_envelope(&record))
                .await
        })
        .await
    }
}

fn validate_nats_config(config: &NatsQueueConfig) -> AppResult<()> {
    if !(config.url.starts_with("nats://")
        || config.url.starts_with("tls://")
        || config.url.starts_with("ws://")
        || config.url.starts_with("wss://"))
    {
        return Err(AppError::InvalidConfig(
            "nats url must start with nats://, tls://, ws://, or wss://".to_owned(),
        ));
    }
    if config.stream_name.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "nats stream_name cannot be empty".to_owned(),
        ));
    }
    if config.subject_prefix.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "nats subject_prefix cannot be empty".to_owned(),
        ));
    }
    if config.consumer_prefix.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "nats consumer_prefix cannot be empty".to_owned(),
        ));
    }
    Ok(())
}
