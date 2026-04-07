use super::*;
use crate::{
    observe_broker_dead_letter, observe_broker_operation, observe_broker_operation_failure,
    observe_broker_retry,
};

pub(super) struct RedisDeadLetterReplay {
    entry_id: String,
}

impl RedisStreamsBrokerAdapter {
    pub async fn connect(config: RedisStreamsQueueConfig) -> AppResult<Self> {
        validate_redis_streams_config(&config)?;
        let client = RedisClient::open(config.url.clone())
            .map_err(|error| redis_error(format!("failed to open redis client: {error}")))?;
        let consumer_name = format!(
            "{}-{}",
            sanitize_consumer_name(&config.consumer_name_prefix),
            Ulid::new().to_string().to_lowercase()
        );

        Ok(Self {
            config,
            inner: InMemoryRuntimeTaskQueue::default(),
            transport: Some(RedisStreamsTransport {
                client,
                consumer_name,
            }),
            routes: RouteCatalog::new(),
            leased: AsyncMutex::new(HashMap::new()),
        })
    }

    fn ensure_route_topology(&self, task: &RuntimeTask) -> AppResult<RedisRouteTopology> {
        self.routes.get_or_insert_with(&task.queue, &task.lane, || {
            build_redis_route_topology(&self.config, &task.queue, &task.lane)
        })
    }

    fn topology_for_route(
        &self,
        route: &(String, String),
    ) -> AppResult<Option<RedisRouteTopology>> {
        self.routes.get(route)
    }

    fn rotate_route(&self, route: &(String, String)) {
        self.routes.rotate(route);
    }

    async fn publish_envelope(
        &self,
        topology: &RedisRouteTopology,
        envelope: &RedisEnvelope,
    ) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        transport.ensure_topology(topology).await?;

        let payload = serde_json::to_string(envelope).map_err(|_| AppError::Internal)?;
        let mut connection = transport.connection().await?;
        let _: String = ::redis::cmd("XADD")
            .arg(&topology.stream_key)
            .arg("*")
            .arg("payload")
            .arg(payload)
            .query_async(&mut connection)
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to publish runtime task to redis stream {}: {error}",
                    topology.stream_key
                ))
            })?;
        Ok(())
    }

    async fn persist_route_registry(&self, topology: &RedisRouteTopology) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let mut connection = transport.connection().await?;
        let payload = serde_json::to_string(&RedisRouteRegistryEntry {
            queue: topology.queue.clone(),
            lane: topology.lane.clone(),
        })
        .map_err(|_| AppError::Internal)?;
        let _: usize = connection
            .sadd(redis_route_registry_key(&self.config), payload)
            .await
            .map_err(|error| {
                redis_error(format!("failed to persist redis route registry: {error}"))
            })?;
        Ok(())
    }

    async fn ensure_registered_routes_loaded(&self) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let mut connection = transport.connection().await?;
        let entries: Vec<String> = connection
            .smembers(redis_route_registry_key(&self.config))
            .await
            .map_err(|error| {
                redis_error(format!("failed to load redis route registry: {error}"))
            })?;

        for payload in entries {
            let entry: RedisRouteRegistryEntry =
                serde_json::from_str(&payload).map_err(|_| AppError::Internal)?;
            let topology = build_redis_route_topology(&self.config, &entry.queue, &entry.lane);
            transport.ensure_topology(&topology).await?;
            self.routes
                .register_route(&entry.queue, &entry.lane, topology)?;
        }

        Ok(())
    }

    async fn promote_due_retries(&self, topology: &RedisRouteTopology) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let mut connection = transport.connection().await?;
        let now = now_ms() as isize;
        let payloads: Vec<String> = ::redis::cmd("ZRANGEBYSCORE")
            .arg(&topology.delayed_key)
            .arg("-inf")
            .arg(now)
            .arg("LIMIT")
            .arg(0)
            .arg(32)
            .query_async(&mut connection)
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to load delayed redis retries from {}: {error}",
                    topology.delayed_key
                ))
            })?;

        for payload in payloads {
            let removed: usize = connection
                .zrem(&topology.delayed_key, &payload)
                .await
                .map_err(|error| {
                    redis_error(format!(
                        "failed to remove delayed redis retry from {}: {error}",
                        topology.delayed_key
                    ))
                })?;
            if removed == 0 {
                continue;
            }

            let delayed: RedisDelayedEnvelope =
                serde_json::from_str(&payload).map_err(|_| AppError::Internal)?;
            self.publish_envelope(topology, &delayed.envelope).await?;
        }

        Ok(())
    }

    async fn reserve_from_broker(
        &self,
        bindings: &[RuntimeRouteBinding],
    ) -> AppResult<Option<RuntimeTaskDelivery>> {
        self.ensure_registered_routes_loaded().await?;
        let routes_snapshot = self.routes.ordered_routes(bindings)?;
        let Some(transport) = &self.transport else {
            return Ok(None);
        };

        for route in routes_snapshot {
            let Some(topology) = self.topology_for_route(&route)? else {
                continue;
            };
            transport.ensure_topology(&topology).await?;
            self.promote_due_retries(&topology).await?;

            if let Some(reclaimed) = self.reclaim_stale_delivery(&topology).await? {
                self.rotate_route(&route);
                return Ok(Some(reclaimed));
            }

            let mut connection = transport.connection().await?;
            let reply: StreamReadReply = connection
                .xread_options(
                    &[topology.stream_key.as_str()],
                    &[">"],
                    &StreamReadOptions::default()
                        .group(
                            topology.consumer_group.as_str(),
                            transport.consumer_name.as_str(),
                        )
                        .count(1)
                        .block(200),
                )
                .await
                .map_err(|error| {
                    redis_error(format!(
                        "failed to reserve runtime task from redis stream {}: {error}",
                        topology.stream_key
                    ))
                })?;

            let Some(key) = reply.keys.first() else {
                continue;
            };
            let Some(message) = key.ids.first() else {
                continue;
            };
            self.rotate_route(&route);

            let payload = message.get::<String>("payload").ok_or(AppError::Internal)?;
            let envelope: RedisEnvelope =
                serde_json::from_str(&payload).map_err(|_| AppError::Internal)?;
            let delivery_id = format!("redis-{}-{}", envelope.task.task_id, message.id);
            let runtime_delivery = RuntimeTaskDelivery {
                delivery_id: delivery_id.clone(),
                attempt: envelope.attempt,
                leased_until: Some(now_ms() + self.config.pending_reclaim_idle_ms),
                last_error: envelope.last_error.clone(),
                task: envelope.task.clone(),
            };
            debug!(
                broker = "redis_streams",
                task_id = %runtime_delivery.task.task_id,
                queue = %runtime_delivery.task.queue,
                lane = %runtime_delivery.task.lane,
                attempt = runtime_delivery.attempt,
                delivery_id = %runtime_delivery.delivery_id,
                leased_until_ms = runtime_delivery.leased_until,
                "reserved runtime task from redis streams"
            );

            self.leased.lock().await.insert(
                delivery_id,
                RedisLeasedDelivery {
                    entry_id: message.id.clone(),
                    topology,
                    delivery: runtime_delivery.clone(),
                },
            );
            return Ok(Some(runtime_delivery));
        }

        Ok(None)
    }

    async fn reclaim_stale_delivery(
        &self,
        topology: &RedisRouteTopology,
    ) -> AppResult<Option<RuntimeTaskDelivery>> {
        let Some(transport) = &self.transport else {
            return Ok(None);
        };
        let mut connection = transport.connection().await?;
        let reply: StreamAutoClaimReply = connection
            .xautoclaim_options(
                &topology.stream_key,
                &topology.consumer_group,
                transport.consumer_name.as_str(),
                self.config.pending_reclaim_idle_ms,
                "0-0",
                StreamAutoClaimOptions::default().count(1),
            )
            .await
            .map_err(|error| {
                observe_broker_operation_failure(
                    "redis_streams",
                    topology.queue.as_str(),
                    topology.lane.as_str(),
                    "reclaim",
                );
                redis_error(format!(
                    "failed to reclaim stale redis delivery from {}: {error}",
                    topology.stream_key
                ))
            })?;

        if !reply.deleted_ids.is_empty() {
            observe_broker_reclaim_orphan_cleanup(
                "redis_streams",
                topology.queue.as_str(),
                topology.lane.as_str(),
                reply.deleted_ids.len() as u64,
            );
            info!(
                broker = "redis_streams",
                queue = %topology.queue,
                lane = %topology.lane,
                stream = %topology.stream_key,
                deleted_orphans = reply.deleted_ids.len(),
                pending_reclaim_idle_ms = self.config.pending_reclaim_idle_ms,
                "redis streams reclaim cleaned orphaned pending entries"
            );
        }

        let Some(message) = reply.claimed.into_iter().next() else {
            return Ok(None);
        };
        let payload = message.get::<String>("payload").ok_or(AppError::Internal)?;
        let envelope: RedisEnvelope =
            serde_json::from_str(&payload).map_err(|_| AppError::Internal)?;
        let delivery_id = format!("redis-{}-{}", envelope.task.task_id, message.id);
        let runtime_delivery = RuntimeTaskDelivery {
            delivery_id: delivery_id.clone(),
            attempt: envelope.attempt,
            leased_until: Some(now_ms() + self.config.pending_reclaim_idle_ms),
            last_error: envelope.last_error.clone(),
            task: envelope.task.clone(),
        };
        observe_broker_reclaim(
            "redis_streams",
            runtime_delivery.task.queue.as_str(),
            runtime_delivery.task.lane.as_str(),
        );
        info!(
            broker = "redis_streams",
            task_id = %runtime_delivery.task.task_id,
            queue = %runtime_delivery.task.queue,
            lane = %runtime_delivery.task.lane,
            attempt = runtime_delivery.attempt,
            delivery_id = %runtime_delivery.delivery_id,
            leased_until_ms = runtime_delivery.leased_until,
            pending_reclaim_idle_ms = self.config.pending_reclaim_idle_ms,
            "reclaimed stale redis streams delivery"
        );
        self.leased.lock().await.insert(
            delivery_id,
            RedisLeasedDelivery {
                entry_id: message.id,
                topology: topology.clone(),
                delivery: runtime_delivery.clone(),
            },
        );
        Ok(Some(runtime_delivery))
    }

    async fn ack_entry(&self, topology: &RedisRouteTopology, entry_id: &str) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let mut connection = transport.connection().await?;
        let _: usize = connection
            .xack(&topology.stream_key, &topology.consumer_group, &[entry_id])
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to ack redis stream entry {}: {error}",
                    entry_id
                ))
            })?;
        let _: usize = connection
            .xdel(&topology.stream_key, &[entry_id])
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to delete acked redis stream entry {}: {error}",
                    entry_id
                ))
            })?;
        Ok(())
    }

    async fn publish_delayed_retry(
        &self,
        topology: &RedisRouteTopology,
        envelope: &RedisEnvelope,
        delay_ms: u64,
    ) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let mut connection = transport.connection().await?;
        let delayed = RedisDelayedEnvelope {
            token: Ulid::new().to_string().to_lowercase(),
            envelope: envelope.clone(),
        };
        let payload = serde_json::to_string(&delayed).map_err(|_| AppError::Internal)?;
        let _: usize = connection
            .zadd(
                &topology.delayed_key,
                payload,
                (now_ms() + delay_ms) as isize,
            )
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to schedule delayed redis retry in {}: {error}",
                    topology.delayed_key
                ))
            })?;
        Ok(())
    }

    async fn publish_dead_letter_record(
        &self,
        topology: &RedisRouteTopology,
        record: &RuntimeDeadLetterRecord,
    ) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let payload = serde_json::to_string(record).map_err(|_| AppError::Internal)?;
        let mut connection = transport.connection().await?;
        let _: String = ::redis::cmd("XADD")
            .arg(&topology.dead_letter_stream_key)
            .arg("*")
            .arg("payload")
            .arg(payload)
            .query_async(&mut connection)
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to publish dead-letter record to redis stream {}: {error}",
                    topology.dead_letter_stream_key
                ))
            })?;
        Ok(())
    }

    async fn load_dead_letters_from_broker(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        self.ensure_registered_routes_loaded().await?;
        let Some(transport) = &self.transport else {
            return Ok(Vec::new());
        };
        let mut records = Vec::new();

        for topology in self.routes.topologies()? {
            let mut connection = transport.connection().await?;
            let reply: StreamRangeReply = connection
                .xrange_all(&topology.dead_letter_stream_key)
                .await
                .map_err(|error| {
                    redis_error(format!(
                        "failed to read redis dead-letter stream {}: {error}",
                        topology.dead_letter_stream_key
                    ))
                })?;

            for message in reply.ids {
                let payload = message.get::<String>("payload").ok_or(AppError::Internal)?;
                let record: RuntimeDeadLetterRecord =
                    serde_json::from_str(&payload).map_err(|_| AppError::Internal)?;
                records.push(record);
            }
        }

        Ok(records)
    }

    async fn find_dead_letter_replay(
        &self,
        delivery_id: &str,
    ) -> AppResult<Option<DeadLetterReplay<RedisRouteTopology, RedisDeadLetterReplay>>> {
        self.ensure_registered_routes_loaded().await?;
        let Some(transport) = &self.transport else {
            return Ok(None);
        };

        for topology in self.routes.topologies()? {
            let mut connection = transport.connection().await?;
            let reply: StreamRangeReply = connection
                .xrange_all(&topology.dead_letter_stream_key)
                .await
                .map_err(|error| {
                    redis_error(format!(
                        "failed to read redis dead-letter stream {}: {error}",
                        topology.dead_letter_stream_key
                    ))
                })?;

            for message in reply.ids {
                let payload = message.get::<String>("payload").ok_or(AppError::Internal)?;
                let record: RuntimeDeadLetterRecord =
                    serde_json::from_str(&payload).map_err(|_| AppError::Internal)?;
                if record.delivery_id == delivery_id {
                    return Ok(Some(DeadLetterReplay {
                        topology,
                        record,
                        stored: RedisDeadLetterReplay {
                            entry_id: message.id,
                        },
                    }));
                }
            }
        }

        Ok(None)
    }

    async fn remove_replayed_dead_letter_entry(
        &self,
        topology: &RedisRouteTopology,
        entry_id: &str,
    ) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            return Ok(());
        };
        let mut connection = transport.connection().await?;
        let _: usize = connection
            .xdel(&topology.dead_letter_stream_key, &[entry_id])
            .await
            .map_err(|error| {
                redis_error(format!(
                    "failed to delete replayed redis dead-letter entry {}: {error}",
                    entry_id
                ))
            })?;
        Ok(())
    }
}

#[async_trait]
impl BrokerDeadLetterStore for RedisStreamsBrokerAdapter {
    type Topology = RedisRouteTopology;
    type Stored = RedisDeadLetterReplay;

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
        self.remove_replayed_dead_letter_entry(&replay.topology, &replay.stored.entry_id)
            .await
    }
}

impl RedisStreamsTransport {
    async fn connection(&self) -> AppResult<MultiplexedConnection> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|error| redis_error(format!("failed to connect to redis: {error}")))
    }

    async fn ensure_topology(&self, topology: &RedisRouteTopology) -> AppResult<()> {
        let mut connection = self.connection().await?;
        let created: Result<String, _> = connection
            .xgroup_create_mkstream(&topology.stream_key, &topology.consumer_group, "0")
            .await;
        if let Err(error) = created {
            if !redis_conflict_error(&error) {
                return Err(redis_error(format!(
                    "failed to create redis consumer group {} on {}: {error}",
                    topology.consumer_group, topology.stream_key
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BrokerAdapter for RedisStreamsBrokerAdapter {
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()> {
        let topology = self.ensure_route_topology(&task)?;
        let queue = task.queue.clone();
        let lane = task.lane.clone();
        if self.transport.is_none() {
            return self.inner.enqueue(task).await;
        }
        self.persist_route_registry(&topology).await?;
        debug!(
            broker = "redis_streams",
            task_id = %task.task_id,
            queue = %task.queue,
            lane = %task.lane,
            stream = %topology.stream_key,
            "publishing runtime task to redis streams"
        );
        self.publish_envelope(
            &topology,
            &RedisEnvelope {
                task,
                attempt: 1,
                last_error: None,
            },
        )
        .await
        .map(|_| {
            observe_broker_operation("redis_streams", queue.as_str(), lane.as_str(), "enqueue");
        })
        .map_err(|error| {
            observe_broker_operation_failure(
                "redis_streams",
                queue.as_str(),
                lane.as_str(),
                "enqueue",
            );
            error
        })
    }

    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery> {
        if self.transport.is_none() {
            return self.inner.reserve(bindings).await;
        }
        match self.reserve_from_broker(bindings).await {
            Ok(Some(delivery)) => {
                observe_broker_operation(
                    "redis_streams",
                    delivery.task.queue.as_str(),
                    delivery.task.lane.as_str(),
                    "reserve",
                );
                Some(delivery)
            }
            Ok(None) => None,
            Err(_) => {
                observe_broker_operation_failure("redis_streams", "*", "*", "reserve");
                None
            }
        }
    }

    async fn ack(&self, delivery_id: &str) -> AppResult<()> {
        if self.transport.is_none() {
            return self.inner.ack(delivery_id).await;
        }
        let leased = self.leased.lock().await.remove(delivery_id);
        if let Some(leased) = leased {
            let queue = leased.delivery.task.queue.clone();
            let lane = leased.delivery.task.lane.clone();
            self.ack_entry(&leased.topology, &leased.entry_id)
                .await
                .map(|_| {
                    observe_broker_operation("redis_streams", queue.as_str(), lane.as_str(), "ack");
                })
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        queue.as_str(),
                        lane.as_str(),
                        "ack",
                    );
                    error
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
        if self.transport.is_none() {
            return self.inner.retry(delivery_id, error, delay_ms).await;
        }

        let Some(leased) = self.leased.lock().await.remove(delivery_id) else {
            observe_broker_operation_failure("redis_streams", "*", "*", "retry");
            return Err(AppError::Internal);
        };

        if leased.delivery.attempt >= leased.delivery.task.retry_policy.max_attempts {
            let record = build_dead_letter_record(&leased.delivery, error);
            observe_broker_retry(
                "redis_streams",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "dead_lettered",
            );
            observe_broker_dead_letter(
                "redis_streams",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "retry_exhausted",
            );
            self.store_dead_letter(&leased.topology, &record)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
            self.ack_entry(&leased.topology, &leased.entry_id)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
            return Ok(RetryDisposition::DeadLettered);
        }

        let envelope = RedisEnvelope {
            task: leased.delivery.task.clone(),
            attempt: leased.delivery.attempt + 1,
            last_error: Some(error.to_owned()),
        };
        if delay_ms == 0 {
            self.publish_envelope(&leased.topology, &envelope)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
        } else {
            self.publish_delayed_retry(&leased.topology, &envelope, delay_ms)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "retry",
                    );
                    error
                })?;
        }
        self.ack_entry(&leased.topology, &leased.entry_id)
            .await
            .map_err(|error| {
                observe_broker_operation_failure(
                    "redis_streams",
                    leased.delivery.task.queue.as_str(),
                    leased.delivery.task.lane.as_str(),
                    "retry",
                );
                error
            })?;
        observe_broker_retry(
            "redis_streams",
            leased.delivery.task.queue.as_str(),
            leased.delivery.task.lane.as_str(),
            "requeued",
        );
        Ok(RetryDisposition::Requeued)
    }

    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()> {
        if self.transport.is_none() {
            return self.inner.reject(delivery_id, error).await;
        }

        let leased = self.leased.lock().await.remove(delivery_id);
        if let Some(leased) = leased {
            let record = build_dead_letter_record(&leased.delivery, error);
            observe_broker_dead_letter(
                "redis_streams",
                leased.delivery.task.queue.as_str(),
                leased.delivery.task.lane.as_str(),
                "rejected",
            );
            self.store_dead_letter(&leased.topology, &record)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "reject",
                    );
                    error
                })?;
            self.ack_entry(&leased.topology, &leased.entry_id)
                .await
                .map_err(|error| {
                    observe_broker_operation_failure(
                        "redis_streams",
                        leased.delivery.task.queue.as_str(),
                        leased.delivery.task.lane.as_str(),
                        "reject",
                    );
                    error
                })?;
        }
        Ok(())
    }

    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        if self.transport.is_none() {
            return self.inner.stats().await;
        }
        self.ensure_registered_routes_loaded().await?;
        let Some(transport) = &self.transport else {
            return Ok(Vec::new());
        };

        let mut stats = Vec::new();
        for topology in self.routes.topologies()? {
            transport.ensure_topology(&topology).await?;
            self.promote_due_retries(&topology).await?;
            let mut connection = transport.connection().await?;
            let groups: StreamInfoGroupsReply = connection
                .xinfo_groups(&topology.stream_key)
                .await
                .map_err(|error| {
                    redis_error(format!(
                        "failed to inspect redis stream groups for {}: {error}",
                        topology.stream_key
                    ))
                })?;
            let stream_len: usize =
                connection
                    .xlen(&topology.stream_key)
                    .await
                    .map_err(|error| {
                        redis_error(format!(
                            "failed to inspect redis stream length for {}: {error}",
                            topology.stream_key
                        ))
                    })?;
            let dead_lettered: usize = connection
                .xlen(&topology.dead_letter_stream_key)
                .await
                .unwrap_or_default();
            let delayed: usize = connection
                .zcard(&topology.delayed_key)
                .await
                .unwrap_or_default();

            let group = groups
                .groups
                .into_iter()
                .find(|group| group.name == topology.consumer_group);
            let leased = group
                .as_ref()
                .map(|group| group.pending)
                .unwrap_or_default();
            let queued = group
                .and_then(|group| group.lag)
                .unwrap_or_else(|| stream_len.saturating_sub(leased))
                + delayed;
            let queue_name = topology.queue.clone();
            let lane_name = topology.lane.clone();

            stats.push(RuntimeQueueStats {
                queue: queue_name.clone(),
                lane: lane_name.clone(),
                queued,
                leased,
                dead_lettered,
            });

            debug!(
                broker = "redis_streams",
                queue = %queue_name,
                lane = %lane_name,
                stream = %topology.stream_key,
                delayed = delayed,
                leased = leased,
                queued = queued,
                dead_lettered = dead_lettered,
                pending_reclaim_idle_ms = self.config.pending_reclaim_idle_ms,
                "redis streams broker stats snapshot"
            );
        }

        Ok(stats)
    }

    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        if self.transport.is_none() {
            return self.inner.dead_letters().await;
        }
        self.load_dead_letter_records().await
    }

    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        if self.transport.is_none() {
            return self.inner.replay_dead_letter(delivery_id).await;
        }
        replay_dead_letter_with_store(
            "redis_streams",
            self,
            delivery_id,
            |topology, record| async move {
                self.publish_envelope(
                    &topology,
                    &RedisEnvelope {
                        task: record.task,
                        attempt: 1,
                        last_error: Some(format!("replayed from dead letter: {}", record.error)),
                    },
                )
                .await
            },
        )
        .await
    }
}

fn validate_redis_streams_config(config: &RedisStreamsQueueConfig) -> AppResult<()> {
    if !config.url.starts_with("redis://") && !config.url.starts_with("rediss://") {
        return Err(AppError::InvalidConfig(
            "redis streams url must start with redis:// or rediss://".to_owned(),
        ));
    }
    if config.stream_prefix.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "redis streams stream_prefix cannot be empty".to_owned(),
        ));
    }
    if config.consumer_group_prefix.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "redis streams consumer_group_prefix cannot be empty".to_owned(),
        ));
    }
    if config.consumer_name_prefix.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "redis streams consumer_name_prefix cannot be empty".to_owned(),
        ));
    }
    Ok(())
}
