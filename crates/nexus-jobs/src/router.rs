use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use nexus_shared::AppResult;

use crate::{
    api::{JobQueryFilter, JobQueryService},
    handlers::{JobHandlerDescriptor, SharedJobHandlerRegistry},
    model::{JobManagementView, JobSnapshot},
};

#[derive(Clone)]
struct JobState {
    query_service: Arc<dyn JobQueryService>,
    handler_registry: SharedJobHandlerRegistry,
}

pub fn build_router(
    query_service: Arc<dyn JobQueryService>,
    handler_registry: SharedJobHandlerRegistry,
) -> Router {
    Router::new()
        .route("/api/v1/jobs", get(list_jobs))
        .route("/api/v1/jobs/handlers", get(list_handlers))
        .route("/api/v1/jobs/management/overview", get(get_job_management))
        .route("/api/v1/jobs/:job_id/history", get(get_job_history))
        .route("/api/v1/jobs/:job_id", get(get_job))
        .with_state(JobState {
            query_service,
            handler_registry,
        })
}

async fn get_job(
    Path(job_id): Path<String>,
    State(state): State<JobState>,
) -> AppResult<Json<JobSnapshot>> {
    Ok(Json(state.query_service.get_job(&job_id).await?))
}

async fn list_jobs(
    Query(filter): Query<JobQueryFilter>,
    State(state): State<JobState>,
) -> AppResult<Json<Vec<JobSnapshot>>> {
    Ok(Json(state.query_service.list_jobs(&filter).await?))
}

async fn list_handlers(
    State(state): State<JobState>,
) -> AppResult<Json<Vec<JobHandlerDescriptor>>> {
    Ok(Json(state.handler_registry.list()))
}

async fn get_job_history(
    Path(job_id): Path<String>,
    State(state): State<JobState>,
) -> AppResult<Json<Vec<crate::model::JobEvent>>> {
    Ok(Json(state.query_service.get_job_history(&job_id).await?))
}

async fn get_job_management(
    Query(filter): Query<JobQueryFilter>,
    State(state): State<JobState>,
) -> AppResult<Json<JobManagementView>> {
    Ok(Json(state.query_service.management_view(&filter).await?))
}
