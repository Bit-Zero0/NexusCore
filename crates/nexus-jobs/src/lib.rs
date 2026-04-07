pub mod api;
pub mod domains;
pub mod handlers;
pub mod model;
pub mod registry;
pub mod router;
pub mod runtime;

pub use api::{
    DefaultJobQueryService, DefaultJobSubmissionValidator, JobPlatformService, JobQueryFilter,
    JobQueryService, JobSubmissionValidator, JobSubmitter,
};
pub use domains::{
    build_oj_judge_job, oj_judge_handler_descriptor, oj_judge_job_handler, OjJudgeJobInput,
    OJ_JOB_NAMESPACE, OJ_JOB_SOURCE_DOMAIN, OJ_JUDGE_JOB_NAME,
};
pub use handlers::{
    InMemoryJobHandlerRegistry, JobDispatchPlan, JobExecutionContext, JobExecutionContract,
    JobHandler, JobHandlerCapabilities, JobHandlerDescriptor, JobHandlerFailure,
    JobHandlerRegistry, JobHandlerResult, SharedJobHandlerRegistry,
};
pub use model::{
    JobDefinition, JobDispatch, JobEvent, JobEventMetadata, JobFailure, JobFailureDisposition,
    JobId, JobJsonPayload, JobManagementSummary, JobManagementView, JobNamespace, JobOrigin,
    JobPayload, JobReceipt, JobResult, JobRetryPolicy, JobRoute, JobSnapshot, JobStatus,
    JobSubmissionFailureSnapshot, JobTimeoutPolicy, JobType,
};
pub use registry::{InMemoryJobDefinitionStore, JobDefinitionStore};
pub use registry::{InMemoryJobEventStore, JobEventStore};
pub use router::build_router;
pub use runtime::{map_job_to_runtime_task, JobRuntimeEventObserver, RuntimeBackedJobSubmitter};
