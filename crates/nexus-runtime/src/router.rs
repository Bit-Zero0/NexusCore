use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};

use nexus_shared::AppResult;
use serde::Deserialize;

use crate::{
    runtime_management_runbooks, RuntimeBrokerManagementRunbookLink, RuntimeBrokerManagementView,
    RuntimeBrokerObservabilityStatus, RuntimeDeadLetterRecord, RuntimeDeadLetterReplayRecord,
    RuntimeNodeStatus, RuntimeQueueReceipt, RuntimeQueueStats, RuntimeSimulationReport,
    RuntimeTask, RuntimeTaskService, RuntimeTaskSnapshot, RuntimeWorkerGroup,
};

#[derive(Clone)]
struct RuntimeState {
    service: Arc<RuntimeTaskService>,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeRouteFilterQuery {
    queue: Option<String>,
    lane: Option<String>,
    group: Option<String>,
    task_id: Option<String>,
    delivery_id: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

pub fn build_router(service: Arc<RuntimeTaskService>) -> Router {
    Router::new()
        .route("/api/v1/runtime/tasks/simulate", post(simulate_task))
        .route("/api/v1/runtime/tasks/schedule", post(schedule_task))
        .route("/api/v1/runtime/tasks/:task_id", get(get_task))
        .route("/api/v1/runtime/node", get(get_runtime_node))
        .route("/api/v1/runtime/broker", get(get_runtime_broker))
        .route(
            "/api/v1/runtime/management/broker",
            get(get_runtime_broker_management),
        )
        .route(
            "/api/v1/runtime/management/runbooks",
            get(get_runtime_management_runbooks),
        )
        .route("/api/v1/runtime/queues/stats", get(get_queue_stats))
        .route("/api/v1/runtime/worker-groups", get(get_worker_groups))
        .route("/api/v1/runtime/queues/dead-letters", get(get_dead_letters))
        .route(
            "/api/v1/runtime/queues/replays",
            get(get_dead_letter_replays),
        )
        .route(
            "/api/v1/runtime/queues/dead-letters/:delivery_id/replay",
            post(replay_dead_letter),
        )
        .with_state(RuntimeState { service })
}

async fn simulate_task(
    State(state): State<RuntimeState>,
    Json(task): Json<RuntimeTask>,
) -> AppResult<Json<RuntimeSimulationReport>> {
    let report = state.service.simulate(task)?;
    Ok(Json(report))
}

async fn schedule_task(
    State(state): State<RuntimeState>,
    Json(task): Json<RuntimeTask>,
) -> AppResult<Json<RuntimeQueueReceipt>> {
    let receipt = state.service.schedule(task).await?;
    Ok(Json(receipt))
}

async fn get_task(
    Path(task_id): Path<String>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<RuntimeTaskSnapshot>> {
    let snapshot = state.service.get_task(&task_id)?;
    Ok(Json(snapshot))
}

async fn get_queue_stats(
    Query(filter): Query<RuntimeRouteFilterQuery>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<Vec<RuntimeQueueStats>>> {
    let stats = state
        .service
        .queue_stats()
        .await?
        .into_iter()
        .filter(|stat| matches_route(stat.queue.as_str(), stat.lane.as_str(), &filter))
        .collect();
    Ok(Json(stats))
}

async fn get_runtime_node(State(state): State<RuntimeState>) -> AppResult<Json<RuntimeNodeStatus>> {
    Ok(Json(state.service.node_status()))
}

async fn get_runtime_broker(
    State(state): State<RuntimeState>,
) -> AppResult<Json<RuntimeBrokerObservabilityStatus>> {
    Ok(Json(state.service.broker_status()))
}

async fn get_runtime_broker_management(
    Query(filter): Query<RuntimeRouteFilterQuery>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<RuntimeBrokerManagementView>> {
    let mut view = state.service.broker_management_view().await?;
    view.queue_stats = view
        .queue_stats
        .into_iter()
        .filter(|stat| matches_route(stat.queue.as_str(), stat.lane.as_str(), &filter))
        .collect();
    let filtered_dead_letters: Vec<_> = view
        .dead_letters
        .into_iter()
        .filter(|record| {
            matches_route(record.queue.as_str(), record.lane.as_str(), &filter)
                && matches_record_identity(
                    record.delivery_id.as_str(),
                    record.task_id.as_str(),
                    &filter,
                )
        })
        .collect();
    let filtered_replays: Vec<_> = view
        .replay_history
        .into_iter()
        .filter(|record| {
            matches_route(record.queue.as_str(), record.lane.as_str(), &filter)
                && matches_record_identity(
                    record.delivery_id.as_str(),
                    record.task_id.as_str(),
                    &filter,
                )
        })
        .collect();
    let replay_history_total = filtered_replays.len();
    view.dead_letters = paginate(filtered_dead_letters.clone(), &filter);
    view.replay_history = paginate(filtered_replays, &filter);
    view.worker_groups = view
        .worker_groups
        .into_iter()
        .filter(|group| matches_group_filter(group, &filter))
        .collect();
    view.summary.queue_count = view.queue_stats.len();
    view.summary.queued = view.queue_stats.iter().map(|item| item.queued).sum();
    view.summary.leased = view.queue_stats.iter().map(|item| item.leased).sum();
    view.summary.dead_lettered = view.queue_stats.iter().map(|item| item.dead_lettered).sum();
    view.summary.dead_letter_records_total = filtered_dead_letters.len();
    view.summary.replay_history_total = replay_history_total;
    view.summary.replayed = view.summary.replay_history_total;
    Ok(Json(view))
}

async fn get_runtime_management_runbooks(
) -> AppResult<Json<Vec<RuntimeBrokerManagementRunbookLink>>> {
    Ok(Json(runtime_management_runbooks()))
}

async fn get_worker_groups(
    Query(filter): Query<RuntimeRouteFilterQuery>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<Vec<RuntimeWorkerGroup>>> {
    let groups = state
        .service
        .worker_groups()
        .into_iter()
        .filter(|group| matches_group_filter(group, &filter))
        .collect();
    Ok(Json(groups))
}

async fn get_dead_letters(
    Query(filter): Query<RuntimeRouteFilterQuery>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<Vec<RuntimeDeadLetterRecord>>> {
    let dead_letters = state
        .service
        .dead_letters()
        .await?
        .into_iter()
        .filter(|record| {
            matches_route(record.queue.as_str(), record.lane.as_str(), &filter)
                && matches_record_identity(
                    record.delivery_id.as_str(),
                    record.task_id.as_str(),
                    &filter,
                )
        })
        .collect();
    Ok(Json(dead_letters))
}

async fn replay_dead_letter(
    Path(delivery_id): Path<String>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<RuntimeQueueReceipt>> {
    Ok(Json(state.service.replay_dead_letter(&delivery_id).await?))
}

async fn get_dead_letter_replays(
    Query(filter): Query<RuntimeRouteFilterQuery>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<Vec<RuntimeDeadLetterReplayRecord>>> {
    let mut records: Vec<_> = state
        .service
        .replay_history()
        .into_iter()
        .filter(|record| {
            matches_route(record.queue.as_str(), record.lane.as_str(), &filter)
                && matches_record_identity(
                    record.delivery_id.as_str(),
                    record.task_id.as_str(),
                    &filter,
                )
        })
        .collect();
    records.sort_by(|left, right| right.replayed_at_ms.cmp(&left.replayed_at_ms));
    Ok(Json(paginate(records, &filter)))
}

fn matches_route(queue: &str, lane: &str, filter: &RuntimeRouteFilterQuery) -> bool {
    filter.queue.as_deref().map_or(true, |value| value == queue)
        && filter.lane.as_deref().map_or(true, |value| value == lane)
}

fn matches_group_filter(group: &RuntimeWorkerGroup, filter: &RuntimeRouteFilterQuery) -> bool {
    let group_matches = filter
        .group
        .as_deref()
        .map_or(true, |value| value == group.name);
    let route_matches = if filter.queue.is_none() && filter.lane.is_none() {
        true
    } else {
        group
            .bindings
            .iter()
            .any(|binding| matches_route(binding.queue.as_str(), binding.lane.as_str(), filter))
    };
    group_matches && route_matches
}

fn matches_record_identity(
    delivery_id: &str,
    task_id: &str,
    filter: &RuntimeRouteFilterQuery,
) -> bool {
    filter
        .delivery_id
        .as_deref()
        .map_or(true, |value| value == delivery_id)
        && filter
            .task_id
            .as_deref()
            .map_or(true, |value| value == task_id)
}

fn paginate<T>(items: Vec<T>, filter: &RuntimeRouteFilterQuery) -> Vec<T> {
    let offset = filter.offset.unwrap_or(0);
    let limit = filter.limit.unwrap_or(50);
    items.into_iter().skip(offset).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        matches_group_filter, matches_record_identity, matches_route, paginate,
        RuntimeRouteFilterQuery,
    };
    use crate::{RuntimeRouteBinding, RuntimeWorkerGroup};

    #[test]
    fn route_filter_matches_expected_queue_and_lane() {
        let filter = RuntimeRouteFilterQuery {
            queue: Some("oj_judge".to_owned()),
            lane: Some("fast".to_owned()),
            group: None,
            task_id: None,
            delivery_id: None,
            limit: None,
            offset: None,
        };

        assert!(matches_route("oj_judge", "fast", &filter));
        assert!(!matches_route("oj_judge", "heavy", &filter));
        assert!(!matches_route("function", "fast", &filter));
    }

    #[test]
    fn worker_group_filter_matches_group_name_and_bindings() {
        let group = RuntimeWorkerGroup {
            name: "oj-fast".to_owned(),
            bindings: vec![RuntimeRouteBinding {
                queue: "oj_judge".to_owned(),
                lane: "fast".to_owned(),
            }],
        };

        assert!(matches_group_filter(
            &group,
            &RuntimeRouteFilterQuery {
                queue: Some("oj_judge".to_owned()),
                lane: Some("fast".to_owned()),
                group: Some("oj-fast".to_owned()),
                task_id: None,
                delivery_id: None,
                limit: None,
                offset: None,
            }
        ));
        assert!(!matches_group_filter(
            &group,
            &RuntimeRouteFilterQuery {
                queue: Some("oj_judge".to_owned()),
                lane: Some("heavy".to_owned()),
                group: None,
                task_id: None,
                delivery_id: None,
                limit: None,
                offset: None,
            }
        ));
    }

    #[test]
    fn record_identity_filter_matches_delivery_and_task() {
        let filter = RuntimeRouteFilterQuery {
            queue: None,
            lane: None,
            group: None,
            task_id: Some("task-1".to_owned()),
            delivery_id: Some("dlv-1".to_owned()),
            limit: None,
            offset: None,
        };

        assert!(matches_record_identity("dlv-1", "task-1", &filter));
        assert!(!matches_record_identity("dlv-2", "task-1", &filter));
        assert!(!matches_record_identity("dlv-1", "task-2", &filter));
    }

    #[test]
    fn pagination_applies_offset_and_limit() {
        let items = vec![1, 2, 3, 4, 5];
        let page = paginate(
            items,
            &RuntimeRouteFilterQuery {
                queue: None,
                lane: None,
                group: None,
                task_id: None,
                delivery_id: None,
                limit: Some(2),
                offset: Some(1),
            },
        );
        assert_eq!(page, vec![2, 3]);
    }
}
