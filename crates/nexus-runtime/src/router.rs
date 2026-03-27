use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};

use nexus_shared::AppResult;
use serde::Deserialize;

use crate::{
    RuntimeDeadLetterRecord, RuntimeNodeStatus, RuntimeQueueReceipt, RuntimeQueueStats,
    RuntimeSimulationReport, RuntimeTask, RuntimeTaskService, RuntimeTaskSnapshot,
    RuntimeWorkerGroup,
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
}

pub fn build_router(service: Arc<RuntimeTaskService>) -> Router {
    Router::new()
        .route("/api/v1/runtime/tasks/simulate", post(simulate_task))
        .route("/api/v1/runtime/tasks/schedule", post(schedule_task))
        .route("/api/v1/runtime/tasks/:task_id", get(get_task))
        .route("/api/v1/runtime/node", get(get_runtime_node))
        .route("/api/v1/runtime/queues/stats", get(get_queue_stats))
        .route("/api/v1/runtime/worker-groups", get(get_worker_groups))
        .route("/api/v1/runtime/queues/dead-letters", get(get_dead_letters))
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
        .filter(|record| matches_route(record.queue.as_str(), record.lane.as_str(), &filter))
        .collect();
    Ok(Json(dead_letters))
}

async fn replay_dead_letter(
    Path(delivery_id): Path<String>,
    State(state): State<RuntimeState>,
) -> AppResult<Json<RuntimeQueueReceipt>> {
    Ok(Json(state.service.replay_dead_letter(&delivery_id).await?))
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

#[cfg(test)]
mod tests {
    use super::{matches_group_filter, matches_route, RuntimeRouteFilterQuery};
    use crate::{RuntimeRouteBinding, RuntimeWorkerGroup};

    #[test]
    fn route_filter_matches_expected_queue_and_lane() {
        let filter = RuntimeRouteFilterQuery {
            queue: Some("oj_judge".to_owned()),
            lane: Some("fast".to_owned()),
            group: None,
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
            }
        ));
        assert!(!matches_group_filter(
            &group,
            &RuntimeRouteFilterQuery {
                queue: Some("oj_judge".to_owned()),
                lane: Some("heavy".to_owned()),
                group: None,
            }
        ));
    }
}
