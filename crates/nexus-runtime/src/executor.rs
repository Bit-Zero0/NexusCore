use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration as StdDuration, Instant, SystemTime},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};
use ulid::Ulid;

use nexus_shared::{AppError, AppResult};

use crate::{
    broker::{EnhancedBrokerCapabilities, RequiredBrokerCapabilities, MEMORY_BROKER_CAPABILITIES},
    judge::{validate_output, CaseJudgeStatus},
    metrics::broker_failure_health_snapshot,
    observe_broker_dead_letter, observe_broker_operation, observe_broker_operation_failure,
    observe_broker_replay, observe_broker_retry,
    planning::{
        resolve_native_cpp_compiler, resolve_python_runtime, resolve_rustc_binary, rustc_mounts,
        RuntimeExecutionBackend, RuntimeExecutionPlan, RuntimeLanguageCatalog,
    },
    protocol::{
        OjJudgeTask, RuntimeJudgeConfig, RuntimeJudgeMethod, RuntimeJudgeMode, RuntimeSandboxKind,
        RuntimeSpjConfig, RuntimeTask, RuntimeTaskPayload,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeExecutionProfile {
    pub language: String,
    pub judge_mode: String,
    pub testcase_count: usize,
    pub total_score: u32,
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExecutionStatus {
    Simulated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCaseExecutionStatus {
    Queued,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCaseSimulationResult {
    pub case_no: u32,
    pub status: RuntimeCaseExecutionStatus,
    pub score: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSimulationReport {
    pub execution_id: String,
    pub task_id: String,
    pub status: RuntimeExecutionStatus,
    pub profile: RuntimeExecutionProfile,
    pub plan: RuntimeExecutionPlan,
    pub case_results: Vec<RuntimeCaseSimulationResult>,
    pub message: String,
}

#[derive(Clone)]
pub struct RuntimeWorker {
    catalog: RuntimeLanguageCatalog,
    work_root: Arc<PathBuf>,
    nsjail_path: Arc<String>,
    seccomp_mode: RuntimeSeccompMode,
    syscall_profile: RuntimeSyscallProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSeccompMode {
    Log,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSyscallFlavor {
    Auto,
    Generic,
    DebianUbuntu,
    Arch,
    RhelLike,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSyscallArch {
    Auto,
    X86_64,
    Aarch64,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSyscallProfile {
    pub flavor: RuntimeSyscallFlavor,
    pub arch: RuntimeSyscallArch,
}

impl RuntimeWorker {
    pub fn new(
        catalog: RuntimeLanguageCatalog,
        work_root: impl Into<PathBuf>,
        nsjail_path: impl Into<String>,
        seccomp_mode: RuntimeSeccompMode,
        syscall_flavor: RuntimeSyscallFlavor,
        syscall_arch: RuntimeSyscallArch,
    ) -> Self {
        Self {
            catalog,
            work_root: Arc::new(work_root.into()),
            nsjail_path: Arc::new(nsjail_path.into()),
            seccomp_mode,
            syscall_profile: resolve_runtime_syscall_profile(syscall_flavor, syscall_arch),
        }
    }

    pub fn simulate(&self, task: RuntimeTask) -> AppResult<RuntimeSimulationReport> {
        match &task.payload {
            RuntimeTaskPayload::OjJudge(payload) => self.simulate_oj_task(task.task_id, payload),
        }
    }

    pub fn prepare(&self, task: RuntimeTask) -> AppResult<PreparedRuntimeArtifacts> {
        match &task.payload {
            RuntimeTaskPayload::OjJudge(payload) => {
                self.prepare_oj_task(task.task_id.clone(), payload)
            }
        }
    }

    fn simulate_oj_task(
        &self,
        task_id: String,
        payload: &OjJudgeTask,
    ) -> AppResult<RuntimeSimulationReport> {
        self.validate_oj_task(payload)?;

        let spec = self.catalog.resolve(&payload.language).ok_or_else(|| {
            AppError::BadRequest(format!(
                "unsupported runtime language: {}",
                payload.language
            ))
        })?;

        let plan = spec.build_plan(payload)?;
        let total_score = payload.testcases.iter().map(|case| case.score).sum();
        let case_results = payload
            .testcases
            .iter()
            .map(|testcase| RuntimeCaseSimulationResult {
                case_no: testcase.case_no,
                status: RuntimeCaseExecutionStatus::Queued,
                score: testcase.score,
                message: format!(
                    "testcase #{} queued for simulated execution",
                    testcase.case_no
                ),
            })
            .collect();

        Ok(RuntimeSimulationReport {
            execution_id: format!("rt-{}", Ulid::new()),
            task_id,
            status: RuntimeExecutionStatus::Simulated,
            profile: RuntimeExecutionProfile {
                language: payload.language.clone(),
                judge_mode: match payload.judge_mode {
                    RuntimeJudgeMode::Acm => "acm".to_owned(),
                    RuntimeJudgeMode::Functional => "functional".to_owned(),
                },
                testcase_count: payload.testcases.len(),
                total_score,
                time_limit_ms: payload.limits.time_limit_ms,
                memory_limit_kb: payload.limits.memory_limit_kb,
            },
            plan,
            case_results,
            message: "runtime simulation completed; task is ready for real execution".to_owned(),
        })
    }

    fn validate_oj_task(&self, payload: &OjJudgeTask) -> AppResult<()> {
        if payload.source_code.trim().is_empty() {
            return Err(AppError::BadRequest(
                "runtime task source_code must not be empty".to_owned(),
            ));
        }

        if payload.limits.time_limit_ms == 0 {
            return Err(AppError::BadRequest(
                "runtime task time_limit_ms must be greater than zero".to_owned(),
            ));
        }

        if payload.limits.memory_limit_kb == 0 {
            return Err(AppError::BadRequest(
                "runtime task memory_limit_kb must be greater than zero".to_owned(),
            ));
        }

        if payload.testcases.is_empty() {
            return Err(AppError::BadRequest(
                "runtime task must contain at least one testcase".to_owned(),
            ));
        }

        let mut seen = std::collections::BTreeSet::new();
        for testcase in &payload.testcases {
            if !seen.insert(testcase.case_no) {
                return Err(AppError::BadRequest(format!(
                    "duplicate runtime testcase case_no: {}",
                    testcase.case_no
                )));
            }
        }

        if matches!(payload.judge_mode, RuntimeJudgeMode::Functional) {
            let signature_exists = payload
                .judge_config
                .as_ref()
                .and_then(|config| config.function_signature.as_ref())
                .is_some();
            if !signature_exists {
                return Err(AppError::BadRequest(
                    "functional runtime task requires function_signature".to_owned(),
                ));
            }
        }

        if matches!(
            payload.sandbox_kind,
            RuntimeSandboxKind::Wasm | RuntimeSandboxKind::NsjailWasm
        ) && payload
            .judge_config
            .as_ref()
            .is_some_and(|config| matches!(config.judge_method, RuntimeJudgeMethod::Spj))
        {
            return Err(AppError::BadRequest(
                "wasm sandbox currently supports validator-based judging only".to_owned(),
            ));
        }

        Ok(())
    }

    fn prepare_oj_task(
        &self,
        task_id: String,
        payload: &OjJudgeTask,
    ) -> AppResult<PreparedRuntimeArtifacts> {
        self.validate_oj_task(payload)?;

        let spec = self.catalog.resolve(&payload.language).ok_or_else(|| {
            AppError::BadRequest(format!(
                "unsupported runtime language: {}",
                payload.language
            ))
        })?;

        let plan = spec.build_plan(payload)?;
        let execution_id = format!("rt-{}", Ulid::new());
        let work_dir = self.work_root.join(&execution_id);
        fs::create_dir_all(&work_dir).map_err(|_| AppError::Internal)?;

        let source_path = work_dir.join(&plan.source_filename);
        fs::write(&source_path, &payload.source_code).map_err(|_| AppError::Internal)?;

        let testcase_dirs = payload
            .testcases
            .iter()
            .map(|testcase| self.prepare_testcase_dir(&work_dir, testcase))
            .collect::<AppResult<Vec<_>>>()?;

        let compile_stage = if plan.compile_required {
            Some(self.build_compile_stage(&work_dir, &plan))
        } else {
            None
        };

        let spj_artifacts = self.build_spj_artifacts(&work_dir, payload.judge_config.as_ref())?;

        let run_stages = testcase_dirs
            .iter()
            .map(|case_dir| self.build_run_stage(&work_dir, &plan, case_dir, payload))
            .collect::<Vec<_>>();

        let manifest_path = work_dir.join("manifest.json");
        let manifest = json!({
            "task_id": task_id,
            "execution_id": execution_id,
            "profile": {
                "language": payload.language,
                "judge_mode": match payload.judge_mode {
                    RuntimeJudgeMode::Acm => "acm",
                    RuntimeJudgeMode::Functional => "functional",
                },
                "testcase_count": payload.testcases.len(),
                "time_limit_ms": payload.limits.time_limit_ms,
                "memory_limit_kb": payload.limits.memory_limit_kb,
            },
            "plan": plan,
            "testcases": testcase_dirs,
        });
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).map_err(|_| AppError::Internal)?,
        )
        .map_err(|_| AppError::Internal)?;

        Ok(PreparedRuntimeArtifacts {
            execution_id,
            task_id,
            status: RuntimeTaskLifecycleStatus::Prepared,
            work_dir: work_dir.display().to_string(),
            source_path: source_path.display().to_string(),
            manifest_path: manifest_path.display().to_string(),
            judge_config: payload.judge_config.clone(),
            cases: testcase_dirs,
            spj_artifacts,
            compile_stage,
            run_stages,
        })
    }

    fn prepare_testcase_dir(
        &self,
        work_dir: &Path,
        testcase: &crate::RuntimeTestcase,
    ) -> AppResult<PreparedRuntimeCaseDir> {
        let case_dir = work_dir
            .join("cases")
            .join(format!("case_{:03}", testcase.case_no));
        fs::create_dir_all(&case_dir).map_err(|_| AppError::Internal)?;

        let input_path = case_dir.join("input.txt");
        let answer_path = case_dir.join("answer.txt");
        let output_path = case_dir.join("output.txt");
        let err_path = case_dir.join("err.txt");

        fs::write(&input_path, &testcase.input).map_err(|_| AppError::Internal)?;
        fs::write(&answer_path, &testcase.expected_output).map_err(|_| AppError::Internal)?;
        fs::write(&output_path, "").map_err(|_| AppError::Internal)?;
        fs::write(&err_path, "").map_err(|_| AppError::Internal)?;

        Ok(PreparedRuntimeCaseDir {
            case_no: testcase.case_no,
            score: testcase.score,
            directory: case_dir.display().to_string(),
            input_path: input_path.display().to_string(),
            answer_path: answer_path.display().to_string(),
            output_path: output_path.display().to_string(),
            stderr_path: err_path.display().to_string(),
        })
    }

    fn build_compile_stage(
        &self,
        work_dir: &Path,
        plan: &RuntimeExecutionPlan,
    ) -> RuntimePreparedStage {
        let stdout_path = work_dir.join("compile.stdout");
        let stderr_path = work_dir.join("compile.stderr");
        let command = match plan.execution_backend {
            RuntimeExecutionBackend::NsjailNative => {
                let seccomp = compiler_seccomp_policy_string(self.syscall_profile);
                wrap_with_nsjail(
                    &self.nsjail_path,
                    work_dir,
                    &plan.compile_command,
                    &seccomp,
                    RuntimeSeccompMode::Log,
                    30,
                    1024 * 1024 * 1024,
                    1024,
                    &plan.readonly_mounts,
                )
            }
            RuntimeExecutionBackend::WasmWasi => wrap_direct_command(
                &plan.compile_command,
                DirectCommandLimits {
                    time_limit_sec: 30,
                    memory_limit_bytes: Some(1024 * 1024 * 1024),
                    pids_limit: Some(1024),
                    file_size_bytes: 32 * 1024 * 1024,
                },
            ),
            RuntimeExecutionBackend::NsjailWasm => {
                let seccomp = compiler_seccomp_policy_string(self.syscall_profile);
                wrap_with_nsjail(
                    &self.nsjail_path,
                    work_dir,
                    &plan.compile_command,
                    &seccomp,
                    RuntimeSeccompMode::Log,
                    30,
                    2 * 1024 * 1024 * 1024,
                    4096,
                    &plan.readonly_mounts,
                )
            }
        };

        RuntimePreparedStage {
            stage_name: "compile".to_owned(),
            score: 0,
            command,
            working_directory: work_dir.display().to_string(),
            stdin_path: None,
            time_limit_ms: 30_000,
            memory_limit_kb: Some(match plan.execution_backend {
                RuntimeExecutionBackend::NsjailWasm => 2 * 1024 * 1024,
                _ => 1024 * 1024,
            }),
            output_limit_bytes: 32 * 1024 * 1024,
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
            readonly_mounts: plan.readonly_mounts.clone(),
        }
    }

    fn build_run_stage(
        &self,
        work_dir: &Path,
        plan: &RuntimeExecutionPlan,
        case_dir: &PreparedRuntimeCaseDir,
        payload: &OjJudgeTask,
    ) -> RuntimePreparedStage {
        let command = match plan.execution_backend {
            RuntimeExecutionBackend::NsjailNative => {
                let seccomp = seccomp_policy_string_with_mode_for_profile(
                    &plan.seccomp_policy,
                    self.seccomp_mode,
                    self.syscall_profile,
                );
                wrap_with_nsjail(
                    &self.nsjail_path,
                    work_dir,
                    &plan.run_command,
                    &seccomp,
                    self.seccomp_mode,
                    payload.limits.time_limit_ms.div_ceil(1000).max(1) + 1,
                    payload.limits.memory_limit_kb * 1024,
                    64,
                    &[],
                )
            }
            RuntimeExecutionBackend::WasmWasi => wrap_direct_command(
                &plan.run_command,
                DirectCommandLimits {
                    time_limit_sec: payload.limits.time_limit_ms.div_ceil(1000).max(1) + 1,
                    memory_limit_bytes: None,
                    pids_limit: None,
                    file_size_bytes: 32 * 1024 * 1024,
                },
            ),
            RuntimeExecutionBackend::NsjailWasm => {
                let seccomp = seccomp_policy_string_with_mode_for_profile(
                    &plan.seccomp_policy,
                    self.seccomp_mode,
                    self.syscall_profile,
                );
                wrap_with_nsjail(
                    &self.nsjail_path,
                    work_dir,
                    &plan.run_command,
                    &seccomp,
                    self.seccomp_mode,
                    payload.limits.time_limit_ms.div_ceil(1000).max(1) + 1,
                    wasm_runtime_host_memory_bytes(payload.limits.memory_limit_kb),
                    1024,
                    &plan.readonly_mounts,
                )
            }
        };

        RuntimePreparedStage {
            stage_name: format!("run_case_{}", case_dir.case_no),
            score: case_dir.score,
            command,
            working_directory: work_dir.display().to_string(),
            stdin_path: Some(case_dir.input_path.clone()),
            time_limit_ms: payload.limits.time_limit_ms,
            memory_limit_kb: Some(payload.limits.memory_limit_kb),
            output_limit_bytes: 32 * 1024 * 1024,
            stdout_path: case_dir.output_path.clone(),
            stderr_path: case_dir.stderr_path.clone(),
            readonly_mounts: Vec::new(),
        }
    }

    fn build_spj_artifacts(
        &self,
        work_dir: &Path,
        judge_config: Option<&RuntimeJudgeConfig>,
    ) -> AppResult<Option<PreparedSpjArtifacts>> {
        let Some(config) = judge_config else {
            return Ok(None);
        };
        if !matches!(config.judge_method, RuntimeJudgeMethod::Spj) {
            return Ok(None);
        }
        let spj = config
            .spj
            .as_ref()
            .ok_or_else(|| AppError::BadRequest("spj judge requires spj config".to_owned()))?;
        let spec = build_spj_execution_spec(spj)?;
        let source_path = work_dir.join(&spec.source_filename);
        fs::write(&source_path, &spj.source_code).map_err(|_| AppError::Internal)?;

        let compile_stage = if let Some(command) = spec.compile_command {
            Some(RuntimePreparedStage {
                stage_name: "spj_compile".to_owned(),
                score: 0,
                command: wrap_with_nsjail(
                    &self.nsjail_path,
                    work_dir,
                    &command,
                    &compiler_seccomp_policy_string(self.syscall_profile),
                    RuntimeSeccompMode::Log,
                    30,
                    1024 * 1024 * 1024,
                    1024,
                    &spec.readonly_mounts,
                ),
                working_directory: work_dir.display().to_string(),
                stdin_path: None,
                time_limit_ms: 30_000,
                memory_limit_kb: Some(1024 * 1024),
                output_limit_bytes: 32 * 1024 * 1024,
                stdout_path: work_dir.join("spj_compile.stdout").display().to_string(),
                stderr_path: work_dir.join("spj_compile.stderr").display().to_string(),
                readonly_mounts: spec.readonly_mounts.clone(),
            })
        } else {
            None
        };

        Ok(Some(PreparedSpjArtifacts {
            language: spec.language,
            source_path: source_path.display().to_string(),
            readonly_mounts: spec.readonly_mounts,
            compile_stage,
            run_command: spec.run_command,
        }))
    }

    async fn execute_spj_case(
        &self,
        artifacts: &PreparedRuntimeArtifacts,
        spj: &PreparedSpjArtifacts,
        case_dir: &PreparedRuntimeCaseDir,
    ) -> AppResult<RuntimeCaseFinalStatus> {
        let stdout_path = PathBuf::from(&case_dir.directory).join("spj.stdout");
        let stderr_path = PathBuf::from(&case_dir.directory).join("spj.stderr");
        let mut args = spj.run_command.clone();
        args.push(path_in_jail(
            Path::new(&artifacts.work_dir),
            Path::new(&case_dir.input_path),
        ));
        args.push(path_in_jail(
            Path::new(&artifacts.work_dir),
            Path::new(&case_dir.output_path),
        ));
        args.push(path_in_jail(
            Path::new(&artifacts.work_dir),
            Path::new(&case_dir.answer_path),
        ));

        let seccomp = seccomp_policy_string_with_mode_for_profile(
            if spj.language == "python" {
                "python_default"
            } else {
                "cpp_default"
            },
            self.seccomp_mode,
            self.syscall_profile,
        );
        let stage = RuntimePreparedStage {
            stage_name: format!("spj_case_{}", case_dir.case_no),
            score: 0,
            command: wrap_with_nsjail(
                &self.nsjail_path,
                Path::new(&artifacts.work_dir),
                &args,
                &seccomp,
                self.seccomp_mode,
                10,
                256 * 1024 * 1024,
                128,
                &spj.readonly_mounts,
            ),
            working_directory: artifacts.work_dir.clone(),
            stdin_path: None,
            time_limit_ms: 10_000,
            memory_limit_kb: Some(256 * 1024),
            output_limit_bytes: 32 * 1024 * 1024,
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
            readonly_mounts: spj.readonly_mounts.clone(),
        };
        let outcome = execute_stage(&stage).await?;
        Ok(match outcome.exit_code {
            Some(0) => RuntimeCaseFinalStatus::Accepted,
            Some(1) => RuntimeCaseFinalStatus::WrongAnswer,
            _ => map_stage_failure_to_case_status(outcome.failure_kind),
        })
    }
}

fn map_stage_failure_to_case_status(
    failure_kind: Option<RuntimeFailureKind>,
) -> RuntimeCaseFinalStatus {
    match failure_kind.unwrap_or(RuntimeFailureKind::RuntimeError) {
        RuntimeFailureKind::TimeLimitExceeded => RuntimeCaseFinalStatus::TimeLimitExceeded,
        RuntimeFailureKind::MemoryLimitExceeded => RuntimeCaseFinalStatus::MemoryLimitExceeded,
        RuntimeFailureKind::OutputLimitExceeded => RuntimeCaseFinalStatus::OutputLimitExceeded,
        RuntimeFailureKind::SecurityViolation => RuntimeCaseFinalStatus::SecurityViolation,
        RuntimeFailureKind::RuntimeError => RuntimeCaseFinalStatus::RuntimeError,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskLifecycleStatus {
    Queued,
    Retrying,
    Preparing,
    Prepared,
    Compiling,
    Running,
    Completed,
    Failed,
    DeadLettered,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePreparedStage {
    pub stage_name: String,
    pub score: u32,
    pub command: Vec<String>,
    pub working_directory: String,
    pub stdin_path: Option<String>,
    pub time_limit_ms: u64,
    pub memory_limit_kb: Option<u64>,
    pub output_limit_bytes: u64,
    pub stdout_path: String,
    pub stderr_path: String,
    pub readonly_mounts: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct DirectCommandLimits {
    time_limit_sec: u64,
    memory_limit_bytes: Option<u64>,
    pids_limit: Option<u32>,
    file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedRuntimeCaseDir {
    pub case_no: u32,
    pub score: u32,
    pub directory: String,
    pub input_path: String,
    pub answer_path: String,
    pub output_path: String,
    pub stderr_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedRuntimeArtifacts {
    pub execution_id: String,
    pub task_id: String,
    pub status: RuntimeTaskLifecycleStatus,
    pub work_dir: String,
    pub source_path: String,
    pub manifest_path: String,
    pub judge_config: Option<RuntimeJudgeConfig>,
    pub cases: Vec<PreparedRuntimeCaseDir>,
    pub spj_artifacts: Option<PreparedSpjArtifacts>,
    pub compile_stage: Option<RuntimePreparedStage>,
    pub run_stages: Vec<RuntimePreparedStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskSnapshot {
    pub task_id: String,
    pub source_domain: String,
    pub queue: String,
    pub lane: String,
    pub status: RuntimeTaskLifecycleStatus,
    pub message: String,
    pub artifacts: Option<PreparedRuntimeArtifacts>,
    pub outcome: Option<RuntimeExecutionOutcome>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueueReceipt {
    pub task_id: String,
    pub queue: String,
    pub lane: String,
    pub status: RuntimeTaskLifecycleStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStageStatus {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFailureKind {
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    SecurityViolation,
    RuntimeError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStageOutcome {
    pub stage_name: String,
    pub status: RuntimeStageStatus,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub failure_kind: Option<RuntimeFailureKind>,
    pub duration_ms: u128,
    pub memory_used_kb: u64,
    pub stdout_size_bytes: u64,
    pub stderr_size_bytes: u64,
    pub stdout_path: String,
    pub stderr_path: String,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCaseFinalStatus {
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    SecurityViolation,
    RuntimeError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCaseOutcome {
    pub case_no: u32,
    pub score: u32,
    pub status: RuntimeCaseFinalStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub memory_used_kb: u64,
    pub stdout_path: String,
    pub stderr_path: String,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeExecutionOutcome {
    pub compile: Option<RuntimeStageOutcome>,
    pub judge_compile: Option<RuntimeStageOutcome>,
    pub cases: Vec<RuntimeCaseOutcome>,
    pub final_status: RuntimeTaskLifecycleStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedSpjArtifacts {
    pub language: String,
    pub source_path: String,
    pub readonly_mounts: Vec<String>,
    pub compile_stage: Option<RuntimePreparedStage>,
    pub run_command: Vec<String>,
}

#[async_trait]
pub trait RuntimeTaskQueue: Send + Sync {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskDelivery {
    pub delivery_id: String,
    pub attempt: u32,
    pub leased_until: Option<u64>,
    pub last_error: Option<String>,
    pub task: RuntimeTask,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuntimeRouteBinding {
    pub queue: String,
    pub lane: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeWorkerGroup {
    pub name: String,
    pub bindings: Vec<RuntimeRouteBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeNodeHealthStatus {
    Healthy,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeNodeStatus {
    pub node_id: String,
    pub started_at_ms: u64,
    pub last_heartbeat_ms: u64,
    pub node_status: RuntimeNodeHealthStatus,
    pub worker_groups: Vec<RuntimeWorkerGroup>,
    pub broker: RuntimeBrokerObservabilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBrokerObservabilityStatus {
    pub broker: String,
    pub required_capabilities: RequiredBrokerCapabilities,
    pub enhanced_capabilities: EnhancedBrokerCapabilities,
    pub ack_wait_ms: Option<u64>,
    pub pending_reclaim_idle_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDeadLetterReplayRecord {
    pub delivery_id: String,
    pub task_id: String,
    pub queue: String,
    pub lane: String,
    pub replayed_at_ms: u64,
    pub status: RuntimeTaskLifecycleStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBrokerManagementSummary {
    pub queue_count: usize,
    pub queued: usize,
    pub leased: usize,
    pub dead_lettered: usize,
    pub replayed: usize,
    pub dead_letter_records_total: usize,
    pub replay_history_total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBrokerHealthState {
    Healthy,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBrokerDegradationReason {
    RecoveryWindowActive,
    PersistentFailuresDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBrokerManagementAlertSeverity {
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBrokerManagementActionKind {
    ObserveRecovery,
    InspectBrokerLogs,
    CheckBrokerConnectivity,
    OpenRunbook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBrokerManagementRunbookLink {
    pub runbook_ref: String,
    pub title: String,
    pub doc_path: String,
    pub section_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBrokerManagementRecommendedAction {
    pub label: String,
    pub action_kind: RuntimeBrokerManagementActionKind,
    pub runbook_ref: Option<String>,
    pub runbook: Option<RuntimeBrokerManagementRunbookLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBrokerManagementAlert {
    pub code: String,
    pub severity: RuntimeBrokerManagementAlertSeverity,
    pub reason: RuntimeBrokerDegradationReason,
    pub message: String,
    pub recommended_action: Option<RuntimeBrokerManagementRecommendedAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBrokerManagementHealth {
    pub status: RuntimeBrokerHealthState,
    pub degradation_reasons: Vec<RuntimeBrokerDegradationReason>,
    pub alerts: Vec<RuntimeBrokerManagementAlert>,
    pub recovery_window_active: bool,
    pub persistent_failures_detected: bool,
    pub last_failure_at_ms: Option<u64>,
    pub recent_failure_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBrokerManagementView {
    pub node: RuntimeNodeStatus,
    pub broker: RuntimeBrokerObservabilityStatus,
    pub health: RuntimeBrokerManagementHealth,
    pub runbooks: Vec<RuntimeBrokerManagementRunbookLink>,
    pub worker_groups: Vec<RuntimeWorkerGroup>,
    pub queue_stats: Vec<RuntimeQueueStats>,
    pub dead_letters: Vec<RuntimeDeadLetterRecord>,
    pub replay_history: Vec<RuntimeDeadLetterReplayRecord>,
    pub summary: RuntimeBrokerManagementSummary,
}

impl RuntimeBrokerObservabilityStatus {
    pub fn from_capability_profile(
        broker: impl Into<String>,
        required_capabilities: RequiredBrokerCapabilities,
        enhanced_capabilities: EnhancedBrokerCapabilities,
    ) -> Self {
        Self {
            broker: broker.into(),
            required_capabilities,
            enhanced_capabilities,
            ack_wait_ms: None,
            pending_reclaim_idle_ms: None,
        }
    }

    pub fn with_ack_wait_ms(mut self, ack_wait_ms: Option<u64>) -> Self {
        self.ack_wait_ms = ack_wait_ms;
        self
    }

    pub fn with_pending_reclaim_idle_ms(mut self, pending_reclaim_idle_ms: Option<u64>) -> Self {
        self.pending_reclaim_idle_ms = pending_reclaim_idle_ms;
        self
    }

    pub fn memory() -> Self {
        Self::from_capability_profile(
            MEMORY_BROKER_CAPABILITIES.broker,
            MEMORY_BROKER_CAPABILITIES.required,
            MEMORY_BROKER_CAPABILITIES.enhanced,
        )
    }
}

fn build_broker_management_health(
    health_snapshot: crate::metrics::BrokerFailureHealthSnapshot,
) -> RuntimeBrokerManagementHealth {
    let mut degradation_reasons = Vec::new();
    let mut alerts = Vec::new();

    if health_snapshot.recovery_window_active {
        degradation_reasons.push(RuntimeBrokerDegradationReason::RecoveryWindowActive);
        alerts.push(RuntimeBrokerManagementAlert {
            code: "broker_recovery_window_active".to_owned(),
            severity: RuntimeBrokerManagementAlertSeverity::Warning,
            reason: RuntimeBrokerDegradationReason::RecoveryWindowActive,
            message: "broker is still inside the recent recovery window after an operation failure"
                .to_owned(),
            recommended_action: Some(RuntimeBrokerManagementRecommendedAction {
                label: "Watch broker recovery and confirm failures stop increasing".to_owned(),
                action_kind: RuntimeBrokerManagementActionKind::ObserveRecovery,
                runbook_ref: Some("p0.broker-recovery-window".to_owned()),
                runbook: runtime_management_runbook("p0.broker-recovery-window"),
            }),
        });
    }

    if health_snapshot.persistent_failures_detected {
        degradation_reasons.push(RuntimeBrokerDegradationReason::PersistentFailuresDetected);
        alerts.push(RuntimeBrokerManagementAlert {
            code: "broker_persistent_failures_detected".to_owned(),
            severity: RuntimeBrokerManagementAlertSeverity::Critical,
            reason: RuntimeBrokerDegradationReason::PersistentFailuresDetected,
            message: format!(
                "broker observed {} failures inside the recent health window",
                health_snapshot.recent_failure_count
            ),
            recommended_action: Some(RuntimeBrokerManagementRecommendedAction {
                label: "Inspect broker logs and connectivity, then review dead-letter replay after recovery"
                    .to_owned(),
                action_kind: RuntimeBrokerManagementActionKind::InspectBrokerLogs,
                runbook_ref: Some("p0.broker-persistent-failures".to_owned()),
                runbook: runtime_management_runbook("p0.broker-persistent-failures"),
            }),
        });
    }

    RuntimeBrokerManagementHealth {
        status: if degradation_reasons.is_empty() {
            RuntimeBrokerHealthState::Healthy
        } else {
            RuntimeBrokerHealthState::Degraded
        },
        degradation_reasons,
        alerts,
        recovery_window_active: health_snapshot.recovery_window_active,
        persistent_failures_detected: health_snapshot.persistent_failures_detected,
        last_failure_at_ms: health_snapshot.last_failure_at_ms,
        recent_failure_count: health_snapshot.recent_failure_count,
    }
}

pub fn runtime_management_runbooks() -> Vec<RuntimeBrokerManagementRunbookLink> {
    vec![
        RuntimeBrokerManagementRunbookLink {
            runbook_ref: "p0.broker-recovery-window".to_owned(),
            title: "Broker recovery window handling".to_owned(),
            doc_path: "P0_生产稳定性与运维闭环_Runbook.md".to_owned(),
            section_ref: "broker-recovery-window".to_owned(),
        },
        RuntimeBrokerManagementRunbookLink {
            runbook_ref: "p0.broker-persistent-failures".to_owned(),
            title: "Broker persistent failure handling".to_owned(),
            doc_path: "P0_生产稳定性与运维闭环_Runbook.md".to_owned(),
            section_ref: "broker-persistent-failures".to_owned(),
        },
    ]
}

fn runtime_management_runbook(runbook_ref: &str) -> Option<RuntimeBrokerManagementRunbookLink> {
    runtime_management_runbooks()
        .into_iter()
        .find(|item| item.runbook_ref == runbook_ref)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDisposition {
    Requeued,
    DeadLettered,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueueStats {
    pub queue: String,
    pub lane: String,
    pub queued: usize,
    pub leased: usize,
    pub dead_lettered: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDeadLetterRecord {
    pub delivery_id: String,
    pub task_id: String,
    pub queue: String,
    pub lane: String,
    pub attempt: u32,
    pub error: String,
    pub dead_lettered_at: u64,
    pub task: RuntimeTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskEvent {
    pub task_id: String,
    pub source_domain: String,
    pub queue: String,
    pub lane: String,
    pub attempt: u32,
    pub submission_id: Option<String>,
    pub problem_id: Option<String>,
    pub user_id: Option<String>,
    pub language: Option<String>,
    pub status: RuntimeTaskLifecycleStatus,
    pub message: String,
    pub execution_id: Option<String>,
    pub outcome: Option<RuntimeExecutionOutcome>,
}

#[async_trait]
pub trait RuntimeEventObserver: Send + Sync {
    async fn on_event(&self, event: RuntimeTaskEvent) -> AppResult<()>;
}

#[allow(dead_code)]
pub struct NoopRuntimeEventObserver;

#[async_trait]
impl RuntimeEventObserver for NoopRuntimeEventObserver {
    async fn on_event(&self, _event: RuntimeTaskEvent) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryRuntimeTaskQueue {
    buckets: Mutex<BTreeMap<(String, String), VecDeque<QueuedRuntimeTask>>>,
    route_order: Mutex<VecDeque<(String, String)>>,
    leased: Mutex<HashMap<String, RuntimeTaskDelivery>>,
    dead_letters: Mutex<Vec<RuntimeDeadLetterRecord>>,
}

#[derive(Debug, Clone)]
struct QueuedRuntimeTask {
    task: RuntimeTask,
    attempt: u32,
    available_at: SystemTime,
    last_error: Option<String>,
}

#[async_trait]
impl RuntimeTaskQueue for InMemoryRuntimeTaskQueue {
    async fn enqueue(&self, task: RuntimeTask) -> AppResult<()> {
        let queue = task.queue.clone();
        let lane = task.lane.clone();
        self.enqueue_internal(QueuedRuntimeTask {
            task,
            attempt: 1,
            available_at: SystemTime::now(),
            last_error: None,
        })?;
        observe_broker_operation("memory", queue.as_str(), lane.as_str(), "enqueue");
        Ok(())
    }

    async fn reserve(&self, bindings: &[RuntimeRouteBinding]) -> Option<RuntimeTaskDelivery> {
        let now = SystemTime::now();
        let routes_snapshot = self.filtered_routes(bindings).ok()?;
        for route in routes_snapshot {
            let mut buckets = self.buckets.lock().ok()?;
            let bucket = buckets.get_mut(&route)?;
            if let Some(front) = bucket.front() {
                if front.available_at > now {
                    continue;
                }
            } else {
                continue;
            }

            let queued = bucket.pop_front()?;
            if bucket.is_empty() {
                buckets.remove(&route);
                drop(buckets);
                self.remove_route(&route);
            } else {
                drop(buckets);
                self.rotate_route(&route);
            }

            let delivery = RuntimeTaskDelivery {
                delivery_id: format!("dlv-{}", queued.task.task_id),
                attempt: queued.attempt,
                leased_until: Some(
                    SystemTime::now()
                        .checked_add(StdDuration::from_secs(30))
                        .and_then(|deadline| deadline.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_millis() as u64)
                        .unwrap_or_default(),
                ),
                last_error: queued.last_error,
                task: queued.task,
            };
            self.leased
                .lock()
                .ok()?
                .insert(delivery.delivery_id.clone(), delivery.clone());
            observe_broker_operation(
                "memory",
                delivery.task.queue.as_str(),
                delivery.task.lane.as_str(),
                "reserve",
            );
            return Some(delivery);
        }
        None
    }

    async fn ack(&self, delivery_id: &str) -> AppResult<()> {
        let removed = self
            .leased
            .lock()
            .map_err(|_| AppError::Internal)?
            .remove(delivery_id);
        if let Some(delivery) = removed {
            observe_broker_operation(
                "memory",
                delivery.task.queue.as_str(),
                delivery.task.lane.as_str(),
                "ack",
            );
        }
        Ok(())
    }

    async fn retry(
        &self,
        delivery_id: &str,
        error: &str,
        delay_ms: u64,
    ) -> AppResult<RetryDisposition> {
        let delivery = self
            .leased
            .lock()
            .map_err(|_| AppError::Internal)?
            .remove(delivery_id)
            .ok_or_else(|| {
                observe_broker_operation_failure("memory", "*", "*", "retry");
                AppError::Internal
            })?;
        if delivery.attempt >= delivery.task.retry_policy.max_attempts {
            let task = delivery.task;
            observe_broker_retry(
                "memory",
                task.queue.as_str(),
                task.lane.as_str(),
                "dead_lettered",
            );
            observe_broker_dead_letter(
                "memory",
                task.queue.as_str(),
                task.lane.as_str(),
                "retry_exhausted",
            );
            self.dead_letters
                .lock()
                .map_err(|_| AppError::Internal)?
                .push(RuntimeDeadLetterRecord {
                    delivery_id: delivery.delivery_id,
                    task_id: task.task_id.clone(),
                    queue: task.queue.clone(),
                    lane: task.lane.clone(),
                    attempt: delivery.attempt,
                    error: error.to_owned(),
                    dead_lettered_at: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|duration| duration.as_millis() as u64)
                        .unwrap_or_default(),
                    task,
                });
            return Ok(RetryDisposition::DeadLettered);
        }
        let queue = delivery.task.queue.clone();
        let lane = delivery.task.lane.clone();
        self.enqueue_internal(QueuedRuntimeTask {
            task: delivery.task,
            attempt: delivery.attempt + 1,
            available_at: SystemTime::now()
                .checked_add(StdDuration::from_millis(delay_ms))
                .unwrap_or(SystemTime::now()),
            last_error: Some(error.to_owned()),
        })?;
        observe_broker_retry("memory", queue.as_str(), lane.as_str(), "requeued");
        Ok(RetryDisposition::Requeued)
    }

    async fn reject(&self, delivery_id: &str, error: &str) -> AppResult<()> {
        let delivery = self
            .leased
            .lock()
            .map_err(|_| AppError::Internal)?
            .remove(delivery_id);
        if let Some(delivery) = delivery {
            let task = delivery.task;
            observe_broker_dead_letter(
                "memory",
                task.queue.as_str(),
                task.lane.as_str(),
                "rejected",
            );
            self.dead_letters
                .lock()
                .map_err(|_| AppError::Internal)?
                .push(RuntimeDeadLetterRecord {
                    delivery_id: delivery.delivery_id,
                    task_id: task.task_id.clone(),
                    queue: task.queue.clone(),
                    lane: task.lane.clone(),
                    attempt: delivery.attempt,
                    error: error.to_owned(),
                    dead_lettered_at: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|duration| duration.as_millis() as u64)
                        .unwrap_or_default(),
                    task,
                });
        }
        Ok(())
    }

    async fn stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        let buckets = self.buckets.lock().map_err(|_| AppError::Internal)?;
        let leased = self.leased.lock().map_err(|_| AppError::Internal)?;
        let dead_letters = self.dead_letters.lock().map_err(|_| AppError::Internal)?;

        let mut stats = BTreeMap::<(String, String), RuntimeQueueStats>::new();
        for ((queue, lane), items) in buckets.iter() {
            stats.insert(
                (queue.clone(), lane.clone()),
                RuntimeQueueStats {
                    queue: queue.clone(),
                    lane: lane.clone(),
                    queued: items.len(),
                    leased: 0,
                    dead_lettered: 0,
                },
            );
        }

        for delivery in leased.values() {
            let key = (delivery.task.queue.clone(), delivery.task.lane.clone());
            let entry = stats.entry(key.clone()).or_insert(RuntimeQueueStats {
                queue: key.0,
                lane: key.1,
                queued: 0,
                leased: 0,
                dead_lettered: 0,
            });
            entry.leased += 1;
        }

        for record in dead_letters.iter() {
            let key = (record.queue.clone(), record.lane.clone());
            let entry = stats.entry(key.clone()).or_insert(RuntimeQueueStats {
                queue: key.0,
                lane: key.1,
                queued: 0,
                leased: 0,
                dead_lettered: 0,
            });
            entry.dead_lettered += 1;
        }

        Ok(stats.into_values().collect())
    }

    async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        Ok(self
            .dead_letters
            .lock()
            .map_err(|_| AppError::Internal)?
            .clone())
    }

    async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        let mut dead_letters = self.dead_letters.lock().map_err(|_| AppError::Internal)?;
        let position = dead_letters
            .iter()
            .position(|record| record.delivery_id == delivery_id)
            .ok_or_else(|| {
                observe_broker_operation_failure("memory", "*", "*", "replay");
                AppError::NotFound(format!("dead letter not found: {delivery_id}"))
            })?;
        let record = dead_letters.remove(position);
        drop(dead_letters);

        self.enqueue_internal(QueuedRuntimeTask {
            task: record.task.clone(),
            attempt: 1,
            available_at: SystemTime::now(),
            last_error: Some(format!("replayed from dead letter: {}", record.error)),
        })?;
        observe_broker_replay(
            "memory",
            record.task.queue.as_str(),
            record.task.lane.as_str(),
        );

        Ok(RuntimeQueueReceipt {
            task_id: record.task.task_id,
            queue: record.task.queue,
            lane: record.task.lane,
            status: RuntimeTaskLifecycleStatus::Queued,
        })
    }
}

impl InMemoryRuntimeTaskQueue {
    fn enqueue_internal(&self, queued: QueuedRuntimeTask) -> AppResult<()> {
        let route = (queued.task.queue.clone(), queued.task.lane.clone());
        let mut buckets = self.buckets.lock().map_err(|_| AppError::Internal)?;
        let bucket = buckets.entry(route.clone()).or_insert_with(VecDeque::new);
        let was_empty = bucket.is_empty();
        bucket.push_back(queued);
        drop(buckets);

        if was_empty {
            self.route_order
                .lock()
                .map_err(|_| AppError::Internal)?
                .push_back(route);
        }
        Ok(())
    }

    fn rotate_route(&self, route: &(String, String)) {
        if let Ok(mut order) = self.route_order.lock() {
            if let Some(position) = order.iter().position(|item| item == route) {
                order.remove(position);
                order.push_back(route.clone());
            }
        }
    }

    fn remove_route(&self, route: &(String, String)) {
        if let Ok(mut order) = self.route_order.lock() {
            if let Some(position) = order.iter().position(|item| item == route) {
                order.remove(position);
            }
        }
    }

    fn filtered_routes(
        &self,
        bindings: &[RuntimeRouteBinding],
    ) -> AppResult<VecDeque<(String, String)>> {
        let order = self.route_order.lock().map_err(|_| AppError::Internal)?;
        if bindings.is_empty() {
            return Ok(order.clone());
        }

        let allowed = bindings
            .iter()
            .map(|binding| (binding.queue.clone(), binding.lane.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        Ok(order
            .iter()
            .filter(|route| allowed.contains(route))
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBrokerBackend {
    Memory,
}

pub fn build_runtime_queue(backend: RuntimeBrokerBackend) -> Arc<dyn RuntimeTaskQueue> {
    match backend {
        RuntimeBrokerBackend::Memory => Arc::new(InMemoryRuntimeTaskQueue::default()),
    }
}

#[derive(Clone)]
pub struct RuntimeTaskService {
    worker: Arc<RuntimeWorker>,
    queue: Arc<dyn RuntimeTaskQueue>,
    broker: RuntimeBrokerObservabilityStatus,
    snapshots: Arc<Mutex<HashMap<String, RuntimeTaskSnapshot>>>,
    observer: Arc<dyn RuntimeEventObserver>,
    worker_groups: Arc<Mutex<Vec<RuntimeWorkerGroup>>>,
    node_id: Arc<Mutex<String>>,
    replay_history: Arc<Mutex<VecDeque<RuntimeDeadLetterReplayRecord>>>,
    started_at_ms: u64,
}

impl RuntimeTaskService {
    pub fn new(worker: Arc<RuntimeWorker>, observer: Arc<dyn RuntimeEventObserver>) -> Self {
        Self::with_queue_and_broker(
            worker,
            build_runtime_queue(RuntimeBrokerBackend::Memory),
            RuntimeBrokerObservabilityStatus::memory(),
            observer,
        )
    }

    pub fn with_queue(
        worker: Arc<RuntimeWorker>,
        queue: Arc<dyn RuntimeTaskQueue>,
        observer: Arc<dyn RuntimeEventObserver>,
    ) -> Self {
        Self::with_queue_and_broker(
            worker,
            queue,
            RuntimeBrokerObservabilityStatus::memory(),
            observer,
        )
    }

    pub fn with_queue_and_broker(
        worker: Arc<RuntimeWorker>,
        queue: Arc<dyn RuntimeTaskQueue>,
        broker: RuntimeBrokerObservabilityStatus,
        observer: Arc<dyn RuntimeEventObserver>,
    ) -> Self {
        Self {
            worker,
            queue,
            broker,
            snapshots: Arc::new(Mutex::new(HashMap::new())),
            observer,
            worker_groups: Arc::new(Mutex::new(Vec::new())),
            node_id: Arc::new(Mutex::new("runtime-node".to_owned())),
            replay_history: Arc::new(Mutex::new(VecDeque::new())),
            started_at_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or_default(),
        }
    }

    pub fn register_node(&self, node_id: impl Into<String>) {
        if let Ok(mut guard) = self.node_id.lock() {
            *guard = node_id.into();
        }
    }

    pub fn start_background_worker(&self) {
        self.start_background_workers(default_runtime_worker_groups());
    }

    pub fn start_background_workers(&self, worker_groups: Vec<RuntimeWorkerGroup>) {
        if let Ok(mut guard) = self.worker_groups.lock() {
            *guard = worker_groups.clone();
        }
        for worker_group in worker_groups {
            self.spawn_worker_group(worker_group);
        }
    }

    fn spawn_worker_group(&self, worker_group: RuntimeWorkerGroup) {
        let queue = self.queue.clone();
        let snapshots = self.snapshots.clone();
        let worker = self.worker.clone();
        let observer = self.observer.clone();
        let worker_group_name = worker_group.name.clone();
        let bindings = worker_group.bindings.clone();
        let binding_labels = bindings
            .iter()
            .map(|binding| format!("{}:{}", binding.queue, binding.lane))
            .collect::<Vec<_>>();

        info!(
            worker_group = %worker_group_name,
            bindings = %binding_labels.join(","),
            "runtime worker group started"
        );

        tokio::spawn(async move {
            loop {
                if let Some(delivery) = queue.reserve(&bindings).await {
                    let task = delivery.task.clone();
                    info!(
                        worker_group = %worker_group_name,
                        task_id = %task.task_id,
                        queue = %task.queue,
                        lane = %task.lane,
                        attempt = delivery.attempt,
                        delivery_id = %delivery.delivery_id,
                        "runtime worker reserved task"
                    );
                    let preparing_snapshot = RuntimeTaskSnapshot {
                        task_id: task.task_id.clone(),
                        source_domain: task.source_domain.clone(),
                        queue: task.queue.clone(),
                        lane: task.lane.clone(),
                        status: RuntimeTaskLifecycleStatus::Preparing,
                        message: "runtime task is preparing execution artifacts".to_owned(),
                        artifacts: None,
                        outcome: None,
                        error: None,
                    };
                    update_snapshot(&snapshots, preparing_snapshot.clone());
                    observe_event(
                        &observer,
                        snapshot_event(&task, delivery.attempt, &preparing_snapshot, None, None),
                    )
                    .await;

                    let result = worker.prepare(task.clone());
                    match result {
                        Ok(artifacts) => {
                            let task_id = task.task_id.clone();
                            let prepared_snapshot = RuntimeTaskSnapshot {
                                task_id: task_id.clone(),
                                source_domain: task.source_domain.clone(),
                                queue: task.queue.clone(),
                                lane: task.lane.clone(),
                                status: RuntimeTaskLifecycleStatus::Prepared,
                                message: "runtime artifacts prepared successfully".to_owned(),
                                artifacts: Some(artifacts.clone()),
                                outcome: None,
                                error: None,
                            };
                            update_snapshot(&snapshots, prepared_snapshot.clone());
                            observe_event(
                                &observer,
                                snapshot_event(
                                    &task,
                                    delivery.attempt,
                                    &prepared_snapshot,
                                    Some(&artifacts),
                                    None,
                                ),
                            )
                            .await;

                            if artifacts.compile_stage.is_some() {
                                let compiling_snapshot = RuntimeTaskSnapshot {
                                    task_id: task_id.clone(),
                                    source_domain: task.source_domain.clone(),
                                    queue: task.queue.clone(),
                                    lane: task.lane.clone(),
                                    status: RuntimeTaskLifecycleStatus::Compiling,
                                    message: "compilation stage started".to_owned(),
                                    artifacts: Some(artifacts.clone()),
                                    outcome: None,
                                    error: None,
                                };
                                update_snapshot(&snapshots, compiling_snapshot.clone());
                                observe_event(
                                    &observer,
                                    snapshot_event(
                                        &task,
                                        delivery.attempt,
                                        &compiling_snapshot,
                                        Some(&artifacts),
                                        None,
                                    ),
                                )
                                .await;
                            } else {
                                let running_snapshot = RuntimeTaskSnapshot {
                                    task_id: task_id.clone(),
                                    source_domain: task.source_domain.clone(),
                                    queue: task.queue.clone(),
                                    lane: task.lane.clone(),
                                    status: RuntimeTaskLifecycleStatus::Running,
                                    message: "interpreter task started".to_owned(),
                                    artifacts: Some(artifacts.clone()),
                                    outcome: None,
                                    error: None,
                                };
                                update_snapshot(&snapshots, running_snapshot.clone());
                                observe_event(
                                    &observer,
                                    snapshot_event(
                                        &task,
                                        delivery.attempt,
                                        &running_snapshot,
                                        Some(&artifacts),
                                        None,
                                    ),
                                )
                                .await;
                            }

                            match worker.execute(&artifacts).await {
                                Ok(outcome) => {
                                    let status = outcome.final_status.clone();
                                    let message = match status {
                                        RuntimeTaskLifecycleStatus::Completed => {
                                            "runtime execution completed successfully"
                                        }
                                        RuntimeTaskLifecycleStatus::Failed => {
                                            "runtime execution failed"
                                        }
                                        _ => "runtime execution finished",
                                    }
                                    .to_owned();

                                    let final_snapshot = RuntimeTaskSnapshot {
                                        task_id,
                                        source_domain: task.source_domain.clone(),
                                        queue: task.queue.clone(),
                                        lane: task.lane.clone(),
                                        status,
                                        message,
                                        artifacts: Some(artifacts),
                                        outcome: Some(outcome),
                                        error: None,
                                    };
                                    update_snapshot(&snapshots, final_snapshot.clone());
                                    observe_event(
                                        &observer,
                                        snapshot_event(
                                            &task,
                                            delivery.attempt,
                                            &final_snapshot,
                                            final_snapshot.artifacts.as_ref(),
                                            final_snapshot.outcome.as_ref(),
                                        ),
                                    )
                                    .await;
                                    info!(
                                        worker_group = %worker_group_name,
                                        task_id = %task.task_id,
                                        queue = %task.queue,
                                        lane = %task.lane,
                                        attempt = delivery.attempt,
                                        final_status = ?final_snapshot.status,
                                        "runtime worker finished task"
                                    );
                                    let _ = queue.ack(&delivery.delivery_id).await;
                                }
                                Err(error) => {
                                    handle_delivery_failure(
                                        &queue,
                                        &observer,
                                        &snapshots,
                                        &task,
                                        delivery.attempt,
                                        &delivery.delivery_id,
                                        Some(artifacts),
                                        format!("runtime execution failed: {error}"),
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(error) => {
                            handle_delivery_failure(
                                &queue,
                                &observer,
                                &snapshots,
                                &task,
                                delivery.attempt,
                                &delivery.delivery_id,
                                None,
                                format!("runtime artifact preparation failed: {error}"),
                            )
                            .await;
                        }
                    };
                } else {
                    sleep(Duration::from_millis(100)).await;
                }
            }
        });
    }

    pub fn simulate(&self, task: RuntimeTask) -> AppResult<RuntimeSimulationReport> {
        self.worker.simulate(task)
    }

    pub async fn schedule(&self, task: RuntimeTask) -> AppResult<RuntimeQueueReceipt> {
        update_snapshot(
            &self.snapshots,
            RuntimeTaskSnapshot {
                task_id: task.task_id.clone(),
                source_domain: task.source_domain.clone(),
                queue: task.queue.clone(),
                lane: task.lane.clone(),
                status: RuntimeTaskLifecycleStatus::Queued,
                message: "runtime task queued".to_owned(),
                artifacts: None,
                outcome: None,
                error: None,
            },
        );
        let observer = self.observer.clone();
        let queued_event = RuntimeTaskEvent {
            task_id: task.task_id.clone(),
            source_domain: task.source_domain.clone(),
            queue: task.queue.clone(),
            lane: task.lane.clone(),
            attempt: 1,
            submission_id: extract_submission_id(&task),
            problem_id: extract_problem_id(&task),
            user_id: extract_user_id(&task),
            language: extract_language(&task),
            status: RuntimeTaskLifecycleStatus::Queued,
            message: "runtime task queued".to_owned(),
            execution_id: None,
            outcome: None,
        };
        debug!(
            task_id = %task.task_id,
            queue = %task.queue,
            lane = %task.lane,
            "runtime task scheduled"
        );
        self.queue.enqueue(task.clone()).await?;
        tokio::spawn(async move {
            observe_event(&observer, queued_event).await;
        });
        Ok(RuntimeQueueReceipt {
            task_id: task.task_id,
            queue: task.queue,
            lane: task.lane,
            status: RuntimeTaskLifecycleStatus::Queued,
        })
    }

    pub fn get_task(&self, task_id: &str) -> AppResult<RuntimeTaskSnapshot> {
        self.snapshots
            .lock()
            .map_err(|_| AppError::Internal)?
            .get(task_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("runtime task not found: {task_id}")))
    }

    pub async fn queue_stats(&self) -> AppResult<Vec<RuntimeQueueStats>> {
        self.queue.stats().await
    }

    pub async fn dead_letters(&self) -> AppResult<Vec<RuntimeDeadLetterRecord>> {
        self.queue.dead_letters().await
    }

    pub async fn replay_dead_letter(&self, delivery_id: &str) -> AppResult<RuntimeQueueReceipt> {
        let receipt = self.queue.replay_dead_letter(delivery_id).await?;
        if let Ok(mut history) = self.replay_history.lock() {
            history.push_back(RuntimeDeadLetterReplayRecord {
                delivery_id: delivery_id.to_owned(),
                task_id: receipt.task_id.clone(),
                queue: receipt.queue.clone(),
                lane: receipt.lane.clone(),
                replayed_at_ms: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or_default(),
                status: receipt.status.clone(),
            });
            while history.len() > 100 {
                history.pop_front();
            }
        }
        Ok(receipt)
    }

    pub fn replay_history(&self) -> Vec<RuntimeDeadLetterReplayRecord> {
        self.replay_history
            .lock()
            .map(|history| history.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn broker_management_view(&self) -> AppResult<RuntimeBrokerManagementView> {
        let queue_stats = self.queue_stats().await?;
        let dead_letters = self.dead_letters().await?;
        let replay_history = self.replay_history();
        let health_snapshot = broker_failure_health_snapshot(self.broker.broker.as_str());
        let health = build_broker_management_health(health_snapshot);
        let summary = RuntimeBrokerManagementSummary {
            queue_count: queue_stats.len(),
            queued: queue_stats.iter().map(|item| item.queued).sum(),
            leased: queue_stats.iter().map(|item| item.leased).sum(),
            dead_lettered: queue_stats.iter().map(|item| item.dead_lettered).sum(),
            replayed: replay_history.len(),
            dead_letter_records_total: dead_letters.len(),
            replay_history_total: replay_history.len(),
        };

        Ok(RuntimeBrokerManagementView {
            node: self.node_status(),
            broker: self.broker_status(),
            health,
            runbooks: runtime_management_runbooks(),
            worker_groups: self.worker_groups(),
            queue_stats,
            dead_letters,
            replay_history,
            summary,
        })
    }

    pub fn worker_groups(&self) -> Vec<RuntimeWorkerGroup> {
        self.worker_groups
            .lock()
            .map(|groups| groups.clone())
            .unwrap_or_default()
    }

    pub fn broker_status(&self) -> RuntimeBrokerObservabilityStatus {
        self.broker.clone()
    }

    pub fn node_status(&self) -> RuntimeNodeStatus {
        let now_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(self.started_at_ms);
        RuntimeNodeStatus {
            node_id: self
                .node_id
                .lock()
                .map(|value| value.clone())
                .unwrap_or_else(|_| "runtime-node".to_owned()),
            started_at_ms: self.started_at_ms,
            last_heartbeat_ms: now_ms,
            node_status: RuntimeNodeHealthStatus::Healthy,
            worker_groups: self.worker_groups(),
            broker: self.broker_status(),
        }
    }
}

#[cfg(test)]
mod management_health_tests {
    use super::{
        build_broker_management_health, RuntimeBrokerDegradationReason, RuntimeBrokerHealthState,
        RuntimeBrokerManagementActionKind, RuntimeBrokerManagementAlertSeverity,
    };
    use crate::metrics::BrokerFailureHealthSnapshot;

    #[test]
    fn broker_management_health_reports_degradation_reasons_and_alerts() {
        let health = build_broker_management_health(BrokerFailureHealthSnapshot {
            last_failure_at_ms: Some(123),
            recent_failure_count: 4,
            recovery_window_active: true,
            persistent_failures_detected: true,
        });

        assert!(matches!(health.status, RuntimeBrokerHealthState::Degraded));
        assert_eq!(health.degradation_reasons.len(), 2);
        assert!(health
            .degradation_reasons
            .iter()
            .any(|reason| matches!(reason, RuntimeBrokerDegradationReason::RecoveryWindowActive)));
        assert!(health.degradation_reasons.iter().any(|reason| matches!(
            reason,
            RuntimeBrokerDegradationReason::PersistentFailuresDetected
        )));
        assert_eq!(health.alerts.len(), 2);
        assert!(health.alerts.iter().any(|alert| matches!(
            alert.severity,
            RuntimeBrokerManagementAlertSeverity::Warning
        )));
        assert!(health.alerts.iter().any(|alert| matches!(
            alert.severity,
            RuntimeBrokerManagementAlertSeverity::Critical
        )));
        assert!(health.alerts.iter().all(|alert| {
            alert.recommended_action.as_ref().is_some_and(|action| {
                !action.label.is_empty()
                    && action
                        .runbook_ref
                        .as_ref()
                        .is_some_and(|value| !value.is_empty())
            })
        }));
        assert!(health.alerts.iter().any(|alert| {
            alert.recommended_action.as_ref().is_some_and(|action| {
                matches!(
                    action.action_kind,
                    RuntimeBrokerManagementActionKind::ObserveRecovery
                )
            })
        }));
        assert!(health.alerts.iter().any(|alert| {
            alert.recommended_action.as_ref().is_some_and(|action| {
                matches!(
                    action.action_kind,
                    RuntimeBrokerManagementActionKind::InspectBrokerLogs
                )
            })
        }));
    }
}

pub fn default_runtime_worker_groups() -> Vec<RuntimeWorkerGroup> {
    vec![
        runtime_worker_group("oj-fast", &[("oj_judge", "fast")]),
        runtime_worker_group("oj-normal", &[("oj_judge", "normal")]),
        runtime_worker_group("oj-heavy", &[("oj_judge", "heavy")]),
        runtime_worker_group("oj-special", &[("oj_judge", "special")]),
    ]
}

fn runtime_worker_group(name: &str, bindings: &[(&str, &str)]) -> RuntimeWorkerGroup {
    RuntimeWorkerGroup {
        name: name.to_owned(),
        bindings: bindings
            .iter()
            .map(|(queue, lane)| RuntimeRouteBinding {
                queue: (*queue).to_owned(),
                lane: (*lane).to_owned(),
            })
            .collect(),
    }
}

impl RuntimeWorker {
    pub async fn execute(
        &self,
        artifacts: &PreparedRuntimeArtifacts,
    ) -> AppResult<RuntimeExecutionOutcome> {
        let compile = if let Some(stage) = &artifacts.compile_stage {
            let outcome = execute_stage(stage).await?;
            if !matches!(outcome.status, RuntimeStageStatus::Succeeded) {
                return Ok(RuntimeExecutionOutcome {
                    compile: Some(outcome),
                    judge_compile: None,
                    cases: Vec::new(),
                    final_status: RuntimeTaskLifecycleStatus::Failed,
                });
            }
            Some(outcome)
        } else {
            None
        };

        let judge_compile = if let Some(spj) = &artifacts.spj_artifacts {
            if let Some(stage) = &spj.compile_stage {
                let outcome = execute_stage(stage).await?;
                if !matches!(outcome.status, RuntimeStageStatus::Succeeded) {
                    return Ok(RuntimeExecutionOutcome {
                        compile,
                        judge_compile: Some(outcome),
                        cases: Vec::new(),
                        final_status: RuntimeTaskLifecycleStatus::Failed,
                    });
                }
                Some(outcome)
            } else {
                None
            }
        } else {
            None
        };

        let mut case_outcomes = Vec::with_capacity(artifacts.run_stages.len());
        for stage in &artifacts.run_stages {
            let outcome = execute_stage(stage).await?;
            let case_no = stage
                .stage_name
                .strip_prefix("run_case_")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or_default();
            let is_success = matches!(outcome.status, RuntimeStageStatus::Succeeded);
            let prepared_case = artifacts
                .cases
                .iter()
                .find(|case| case.case_no == case_no)
                .ok_or(AppError::Internal)?;
            let case_status = if !is_success {
                map_stage_failure_to_case_status(outcome.failure_kind)
            } else if let Some(spj_artifacts) = &artifacts.spj_artifacts {
                self.execute_spj_case(artifacts, spj_artifacts, prepared_case)
                    .await?
            } else {
                let user_output = fs::read_to_string(&prepared_case.output_path)
                    .map_err(|_| AppError::Internal)?;
                match validate_output(
                    &user_output,
                    &fs::read_to_string(&prepared_case.answer_path)
                        .map_err(|_| AppError::Internal)?,
                    artifacts.judge_config.as_ref(),
                ) {
                    CaseJudgeStatus::Accepted => RuntimeCaseFinalStatus::Accepted,
                    CaseJudgeStatus::WrongAnswer => RuntimeCaseFinalStatus::WrongAnswer,
                }
            };
            let case_outcome = RuntimeCaseOutcome {
                case_no,
                score: stage.score,
                status: case_status,
                exit_code: outcome.exit_code,
                duration_ms: outcome.duration_ms,
                memory_used_kb: outcome.memory_used_kb,
                stdout_path: outcome.stdout_path.clone(),
                stderr_path: outcome.stderr_path.clone(),
                stdout_excerpt: outcome.stdout_excerpt.clone(),
                stderr_excerpt: outcome.stderr_excerpt.clone(),
            };
            case_outcomes.push(case_outcome);
            if !matches!(
                case_outcomes.last().map(|case| &case.status),
                Some(RuntimeCaseFinalStatus::Accepted)
            ) {
                return Ok(RuntimeExecutionOutcome {
                    compile,
                    judge_compile,
                    cases: case_outcomes,
                    final_status: RuntimeTaskLifecycleStatus::Failed,
                });
            }
        }

        Ok(RuntimeExecutionOutcome {
            compile,
            judge_compile,
            cases: case_outcomes,
            final_status: RuntimeTaskLifecycleStatus::Completed,
        })
    }
}

fn snapshot_event(
    task: &RuntimeTask,
    attempt: u32,
    snapshot: &RuntimeTaskSnapshot,
    artifacts: Option<&PreparedRuntimeArtifacts>,
    outcome: Option<&RuntimeExecutionOutcome>,
) -> RuntimeTaskEvent {
    RuntimeTaskEvent {
        task_id: snapshot.task_id.clone(),
        source_domain: snapshot.source_domain.clone(),
        queue: snapshot.queue.clone(),
        lane: snapshot.lane.clone(),
        attempt,
        submission_id: extract_submission_id(task),
        problem_id: extract_problem_id(task),
        user_id: extract_user_id(task),
        language: extract_language(task),
        status: snapshot.status.clone(),
        message: snapshot.message.clone(),
        execution_id: artifacts.map(|item| item.execution_id.clone()),
        outcome: outcome.cloned(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_delivery_failure(
    queue: &Arc<dyn RuntimeTaskQueue>,
    observer: &Arc<dyn RuntimeEventObserver>,
    snapshots: &Arc<Mutex<HashMap<String, RuntimeTaskSnapshot>>>,
    task: &RuntimeTask,
    attempt: u32,
    delivery_id: &str,
    artifacts: Option<PreparedRuntimeArtifacts>,
    error_message: String,
) {
    let disposition = queue
        .retry(
            delivery_id,
            &error_message,
            task.retry_policy.retry_delay_ms,
        )
        .await
        .unwrap_or(RetryDisposition::DeadLettered);
    warn!(
        task_id = %task.task_id,
        queue = %task.queue,
        lane = %task.lane,
        attempt,
        disposition = ?disposition,
        error = %error_message,
        "runtime delivery failed"
    );

    let (status, message) = match disposition {
        RetryDisposition::Requeued => (
            RuntimeTaskLifecycleStatus::Retrying,
            format!("runtime task retry scheduled: {error_message}"),
        ),
        RetryDisposition::DeadLettered => (
            RuntimeTaskLifecycleStatus::DeadLettered,
            format!("runtime task moved to dead letter: {error_message}"),
        ),
    };

    let snapshot = RuntimeTaskSnapshot {
        task_id: task.task_id.clone(),
        source_domain: task.source_domain.clone(),
        queue: task.queue.clone(),
        lane: task.lane.clone(),
        status,
        message,
        artifacts,
        outcome: None,
        error: Some(error_message),
    };
    update_snapshot(snapshots, snapshot.clone());
    observe_event(
        observer,
        snapshot_event(task, attempt, &snapshot, snapshot.artifacts.as_ref(), None),
    )
    .await;
}

fn extract_submission_id(task: &RuntimeTask) -> Option<String> {
    match &task.payload {
        RuntimeTaskPayload::OjJudge(payload) => Some(payload.submission_id.0.clone()),
    }
}

fn extract_problem_id(task: &RuntimeTask) -> Option<String> {
    match &task.payload {
        RuntimeTaskPayload::OjJudge(payload) => Some(payload.problem_id.0.clone()),
    }
}

fn extract_user_id(task: &RuntimeTask) -> Option<String> {
    match &task.payload {
        RuntimeTaskPayload::OjJudge(payload) => Some(payload.user_id.0.clone()),
    }
}

fn extract_language(task: &RuntimeTask) -> Option<String> {
    match &task.payload {
        RuntimeTaskPayload::OjJudge(payload) => Some(payload.language.clone()),
    }
}

async fn observe_event(observer: &Arc<dyn RuntimeEventObserver>, event: RuntimeTaskEvent) {
    if let Err(error) = observer.on_event(event).await {
        warn!(error = %error, "runtime observer failed to handle event");
    }
}

async fn execute_stage(stage: &RuntimePreparedStage) -> AppResult<RuntimeStageOutcome> {
    let Some(program) = stage.command.first() else {
        return Err(AppError::BadRequest(
            "runtime stage command is empty".to_owned(),
        ));
    };

    let stdout_file = fs::File::create(&stage.stdout_path).map_err(|err| {
        AppError::BadRequest(format!(
            "failed to prepare stdout for stage {}: {err}",
            stage.stage_name
        ))
    })?;
    let stderr_file = fs::File::create(&stage.stderr_path).map_err(|err| {
        AppError::BadRequest(format!(
            "failed to prepare stderr for stage {}: {err}",
            stage.stage_name
        ))
    })?;

    let mut command = Command::new(program);
    command
        .args(&stage.command[1..])
        .current_dir(&stage.working_directory)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", "/tmp")
        .env("TMPDIR", "/tmp")
        .stdout(std::process::Stdio::from(stdout_file))
        .stderr(std::process::Stdio::from(stderr_file));

    if let Some(stdin_path) = &stage.stdin_path {
        let stdin_file = fs::File::open(stdin_path).map_err(|err| {
            AppError::BadRequest(format!(
                "failed to open stdin for stage {}: {err}",
                stage.stage_name
            ))
        })?;
        command.stdin(std::process::Stdio::from(stdin_file));
    }

    let started_at = Instant::now();
    let mut child = command.spawn().map_err(|err| {
        AppError::BadRequest(format!(
            "failed to execute stage {}: {err}",
            stage.stage_name
        ))
    })?;
    let memory_used_kb = monitor_peak_rss_kb(&mut child).await;
    let status = child.wait().await.map_err(|err| {
        AppError::BadRequest(format!(
            "failed to wait for stage {}: {err}",
            stage.stage_name
        ))
    })?;
    let duration_ms = started_at.elapsed().as_millis();

    let stdout_excerpt = read_excerpt(&stage.stdout_path);
    let stderr_excerpt = read_excerpt(&stage.stderr_path);
    let stdout_size_bytes = file_size_bytes(&stage.stdout_path);
    let stderr_size_bytes = file_size_bytes(&stage.stderr_path);
    let status_kind = if status.success() {
        RuntimeStageStatus::Succeeded
    } else {
        RuntimeStageStatus::Failed
    };
    let signal = status.signal();
    let failure_kind = if matches!(status_kind, RuntimeStageStatus::Succeeded) {
        None
    } else {
        classify_stage_failure(
            stage,
            status.code(),
            signal,
            duration_ms,
            memory_used_kb,
            stdout_size_bytes,
            stderr_size_bytes,
            &stdout_excerpt,
            &stderr_excerpt,
        )
    };

    Ok(RuntimeStageOutcome {
        stage_name: stage.stage_name.clone(),
        status: status_kind,
        exit_code: status.code(),
        signal,
        failure_kind,
        duration_ms,
        memory_used_kb,
        stdout_size_bytes,
        stderr_size_bytes,
        stdout_path: stage.stdout_path.clone(),
        stderr_path: stage.stderr_path.clone(),
        stdout_excerpt,
        stderr_excerpt,
    })
}

async fn monitor_peak_rss_kb(child: &mut tokio::process::Child) -> u64 {
    let Some(pid) = child.id() else {
        return 0;
    };

    let status_path = format!("/proc/{pid}/status");
    let mut peak_kb = 0;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                peak_kb = peak_kb.max(read_process_rss_kb(&status_path));
                break;
            }
            Ok(None) => {
                peak_kb = peak_kb.max(read_process_rss_kb(&status_path));
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(_) => break,
        }
    }

    peak_kb
}

fn read_process_rss_kb(status_path: &str) -> u64 {
    let Ok(status) = fs::read_to_string(status_path) else {
        return 0;
    };

    status
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix("VmRSS:")?.trim();
            let kb = value.strip_suffix(" kB").unwrap_or(value).trim();
            kb.parse::<u64>().ok()
        })
        .unwrap_or(0)
}

fn read_excerpt(path: &str) -> String {
    let Ok(content) = fs::read_to_string(path) else {
        return String::new();
    };
    let trimmed = content.trim();
    if trimmed.len() <= 400 {
        trimmed.to_owned()
    } else {
        for marker in [
            "/etc/",
            "/proc/",
            "../",
            "include_bytes!",
            "read-only file system",
            "network is unreachable",
            "permission denied",
            "operation not permitted",
            "file too large",
            "out of memory",
            "cannot allocate memory",
        ] {
            if let Some(index) = trimmed.find(marker) {
                let start = index.saturating_sub(180);
                let end = (index + marker.len() + 180).min(trimmed.len());
                let snippet = &trimmed[start..end];
                return if start > 0 || end < trimmed.len() {
                    format!("...{snippet}...")
                } else {
                    snippet.to_owned()
                };
            }
        }

        let head = &trimmed[..200];
        let start = trimmed.len().saturating_sub(200);
        let tail = &trimmed[start..];
        format!("{head}\n...\n{tail}")
    }
}

fn file_size_bytes(path: &str) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn classify_stage_failure(
    stage: &RuntimePreparedStage,
    exit_code: Option<i32>,
    signal: Option<i32>,
    duration_ms: u128,
    memory_used_kb: u64,
    stdout_size_bytes: u64,
    stderr_size_bytes: u64,
    stdout_excerpt: &str,
    stderr_excerpt: &str,
) -> Option<RuntimeFailureKind> {
    let stdout_lower = stdout_excerpt.to_lowercase();
    let stderr_lower = stderr_excerpt.to_lowercase();
    let combined = format!("{stdout_lower}\n{stderr_lower}");
    let suspicious_host_path_probe = stage.stage_name.contains("compile")
        && (combined.contains("/etc/") || combined.contains("/proc/") || combined.contains("../"))
        && (combined.contains("no such file")
            || combined.contains("permission denied")
            || combined.contains("operation not permitted"));

    if combined.contains("seccomp")
        || combined.contains("operation not permitted")
        || combined.contains("permission denied")
        || combined.contains("network is unreachable")
        || combined.contains("read-only file system")
        || combined.contains("bad system call")
        || signal == Some(31)
        || exit_code == Some(159)
        || suspicious_host_path_probe
    {
        return Some(RuntimeFailureKind::SecurityViolation);
    }

    if signal == Some(25)
        || exit_code == Some(153)
        || stdout_size_bytes >= stage.output_limit_bytes
        || stderr_size_bytes >= stage.output_limit_bytes
        || combined.contains("file too large")
    {
        return Some(RuntimeFailureKind::OutputLimitExceeded);
    }

    let hit_time_limit = duration_ms >= u128::from(stage.time_limit_ms)
        || exit_code == Some(124)
        || combined.contains("timed out")
        || combined.contains("timeout");
    if hit_time_limit {
        return Some(RuntimeFailureKind::TimeLimitExceeded);
    }

    let hit_memory_limit = stage
        .memory_limit_kb
        .is_some_and(|limit| memory_used_kb >= limit.saturating_mul(95) / 100)
        || signal == Some(11)
        || exit_code == Some(139)
        || combined.contains("out of memory")
        || combined.contains("cannot allocate memory")
        || combined.contains("memory allocation")
        || combined.contains("memory limit");
    if hit_memory_limit {
        return Some(RuntimeFailureKind::MemoryLimitExceeded);
    }

    Some(RuntimeFailureKind::RuntimeError)
}

fn update_snapshot(
    snapshots: &Arc<Mutex<HashMap<String, RuntimeTaskSnapshot>>>,
    snapshot: RuntimeTaskSnapshot,
) {
    if let Ok(mut guard) = snapshots.lock() {
        guard.insert(snapshot.task_id.clone(), snapshot);
    }
}

#[allow(clippy::too_many_arguments)]
fn wrap_with_nsjail(
    nsjail_path: &str,
    work_dir: &Path,
    command: &[String],
    seccomp_string: &str,
    seccomp_mode: RuntimeSeccompMode,
    time_limit_sec: u64,
    memory_limit_bytes: u64,
    pids_limit: u32,
    extra_ro_mounts: &[String],
) -> Vec<String> {
    let mut args = vec![
        nsjail_path.to_owned(),
        "-Mo".to_owned(),
        "-Q".to_owned(),
        "--chroot".to_owned(),
        work_dir.display().to_string(),
        "--user".to_owned(),
        "65534".to_owned(),
        "--group".to_owned(),
        "65534".to_owned(),
        "--rw".to_owned(),
        "--cwd".to_owned(),
        "/".to_owned(),
    ];

    for mount in ["/lib", "/lib64", "/usr", "/bin", "/etc/alternatives"] {
        if Path::new(mount).exists() {
            args.push("-R".to_owned());
            args.push(mount.to_owned());
        }
    }

    for mount in extra_ro_mounts {
        if Path::new(mount).exists() {
            args.push("-R".to_owned());
            args.push(mount.clone());
        }
    }

    args.extend([
        "-T".to_owned(),
        "/dev".to_owned(),
        "-T".to_owned(),
        "/tmp".to_owned(),
        "--use_cgroupv2".to_owned(),
        "--time_limit".to_owned(),
        time_limit_sec.to_string(),
        "--cgroup_mem_max".to_owned(),
        memory_limit_bytes.to_string(),
        "--rlimit_as".to_owned(),
        (memory_limit_bytes / 1024 / 1024).max(1).to_string(),
        "--cgroup_pids_max".to_owned(),
        pids_limit.to_string(),
        "--rlimit_stack".to_owned(),
        "64".to_owned(),
        "--rlimit_fsize".to_owned(),
        "32".to_owned(),
        "--seccomp_string".to_owned(),
        seccomp_string.to_owned(),
    ]);
    if matches!(seccomp_mode, RuntimeSeccompMode::Log) {
        args.push("--seccomp_log".to_owned());
    }

    args.push("--".to_owned());
    args.push("/usr/bin/env".to_owned());
    args.push("-i".to_owned());
    args.push("PATH=/usr/bin:/bin:/usr/sbin:/sbin".to_owned());
    args.push("HOME=/tmp".to_owned());
    args.push("TMPDIR=/tmp".to_owned());
    args.extend(
        command
            .iter()
            .map(|part| path_in_jail(work_dir, Path::new(part))),
    );
    args
}

fn wrap_direct_command(command: &[String], limits: DirectCommandLimits) -> Vec<String> {
    let mut args = vec![
        "/usr/bin/timeout".to_owned(),
        "--signal=KILL".to_owned(),
        format!("{}s", limits.time_limit_sec),
        "/usr/bin/prlimit".to_owned(),
        format!("--cpu={}", limits.time_limit_sec),
    ];
    if let Some(memory_limit_bytes) = limits.memory_limit_bytes {
        args.push(format!("--as={memory_limit_bytes}"));
    }
    if let Some(pids_limit) = limits.pids_limit {
        args.push(format!("--nproc={pids_limit}"));
    }
    args.extend([
        format!("--fsize={}", limits.file_size_bytes),
        "--".to_owned(),
    ]);
    args.extend(command.iter().cloned());
    args
}

fn wasm_runtime_host_memory_bytes(memory_limit_kb: u64) -> u64 {
    (memory_limit_kb * 1024 * 8).max(8 * 1024 * 1024 * 1024)
}

fn path_in_jail(work_dir: &Path, path: &Path) -> String {
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(work_dir) {
            return format!("/{}", relative.display());
        }
        return path.display().to_string();
    }

    path.display().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeccompProfile {
    CppNativeRuntime,
    RustNativeRuntime,
    PythonRuntime,
    WasmtimeRuntime,
    Compiler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyscallGroup {
    RuntimeCore,
    RuntimeClock,
    CompilerClock,
    RuntimePositionalRead,
    CompilerPositionalRead,
    RuntimeVectorIo,
    CompilerVectorIo,
    RuntimeFileStatCompat,
    CompilerFileStatCompat,
    FileOpenCompat,
    RuntimeProcessLifecycleCompat,
    CompilerProcessLifecycleCompat,
    RuntimeThreadCompat,
    CompilerThreadCompat,
    SignalCompat,
    CppRuntimeExtras,
    RustRuntimeExtras,
    PythonRuntimeExtras,
    WasmtimeRuntimeExtras,
    CompilerExec,
    CompilerExtras,
}

fn seccomp_profile_for_policy(policy: &str) -> SeccompProfile {
    match policy {
        "python_default" => SeccompProfile::PythonRuntime,
        "wasm_default" => SeccompProfile::WasmtimeRuntime,
        "rust_native_default" => SeccompProfile::RustNativeRuntime,
        "cpp_native_default" | "cpp_default" => SeccompProfile::CppNativeRuntime,
        "compiler" => SeccompProfile::Compiler,
        _ => SeccompProfile::CppNativeRuntime,
    }
}

#[cfg(test)]
fn seccomp_policy_string_with_mode(
    policy: &str,
    mode: RuntimeSeccompMode,
    flavor: RuntimeSyscallFlavor,
) -> String {
    seccomp_policy_string_with_mode_and_arch(policy, mode, flavor, RuntimeSyscallArch::Auto)
}

fn seccomp_policy_string_with_mode_for_profile(
    policy: &str,
    mode: RuntimeSeccompMode,
    syscall_profile: RuntimeSyscallProfile,
) -> String {
    build_seccomp_policy(seccomp_profile_for_policy(policy), mode, syscall_profile)
}

#[cfg(test)]
fn seccomp_policy_string_with_mode_and_arch(
    policy: &str,
    mode: RuntimeSeccompMode,
    flavor: RuntimeSyscallFlavor,
    arch: RuntimeSyscallArch,
) -> String {
    build_seccomp_policy(
        seccomp_profile_for_policy(policy),
        mode,
        RuntimeSyscallProfile { flavor, arch },
    )
}

fn compiler_seccomp_policy_string(syscall_profile: RuntimeSyscallProfile) -> String {
    build_seccomp_policy(
        SeccompProfile::Compiler,
        RuntimeSeccompMode::Log,
        syscall_profile,
    )
}

fn build_seccomp_policy(
    profile: SeccompProfile,
    mode: RuntimeSeccompMode,
    syscall_profile: RuntimeSyscallProfile,
) -> String {
    let syscalls = seccomp_profile_syscalls(profile, syscall_profile);
    let default_action = match mode {
        RuntimeSeccompMode::Log => "LOG",
        RuntimeSeccompMode::Kill => "KILL",
    };
    format!(
        "POLICY def {{ ALLOW {{ {} }} }} USE def DEFAULT {}",
        syscalls.join(", "),
        default_action
    )
}

fn seccomp_profile_groups(profile: SeccompProfile) -> &'static [SyscallGroup] {
    match profile {
        SeccompProfile::CppNativeRuntime => &[
            SyscallGroup::RuntimeCore,
            SyscallGroup::RuntimeClock,
            SyscallGroup::RuntimePositionalRead,
            SyscallGroup::RuntimeVectorIo,
            SyscallGroup::RuntimeFileStatCompat,
            SyscallGroup::FileOpenCompat,
            SyscallGroup::RuntimeProcessLifecycleCompat,
            SyscallGroup::SignalCompat,
            SyscallGroup::CppRuntimeExtras,
        ],
        SeccompProfile::RustNativeRuntime => &[
            SyscallGroup::RuntimeCore,
            SyscallGroup::RuntimeClock,
            SyscallGroup::RuntimePositionalRead,
            SyscallGroup::RuntimeVectorIo,
            SyscallGroup::RuntimeFileStatCompat,
            SyscallGroup::FileOpenCompat,
            SyscallGroup::RuntimeProcessLifecycleCompat,
            SyscallGroup::SignalCompat,
            SyscallGroup::RustRuntimeExtras,
        ],
        SeccompProfile::PythonRuntime => &[
            SyscallGroup::RuntimeCore,
            SyscallGroup::RuntimeClock,
            SyscallGroup::RuntimePositionalRead,
            SyscallGroup::RuntimeVectorIo,
            SyscallGroup::RuntimeFileStatCompat,
            SyscallGroup::FileOpenCompat,
            SyscallGroup::RuntimeProcessLifecycleCompat,
            SyscallGroup::RuntimeThreadCompat,
            SyscallGroup::SignalCompat,
            SyscallGroup::PythonRuntimeExtras,
        ],
        SeccompProfile::WasmtimeRuntime => &[
            SyscallGroup::RuntimeCore,
            SyscallGroup::RuntimeClock,
            SyscallGroup::RuntimePositionalRead,
            SyscallGroup::RuntimeVectorIo,
            SyscallGroup::RuntimeFileStatCompat,
            SyscallGroup::FileOpenCompat,
            SyscallGroup::RuntimeProcessLifecycleCompat,
            SyscallGroup::RuntimeThreadCompat,
            SyscallGroup::SignalCompat,
            SyscallGroup::RustRuntimeExtras,
            SyscallGroup::WasmtimeRuntimeExtras,
        ],
        SeccompProfile::Compiler => &[
            SyscallGroup::RuntimeCore,
            SyscallGroup::CompilerClock,
            SyscallGroup::CompilerPositionalRead,
            SyscallGroup::CompilerVectorIo,
            SyscallGroup::CompilerFileStatCompat,
            SyscallGroup::FileOpenCompat,
            SyscallGroup::CompilerProcessLifecycleCompat,
            SyscallGroup::CompilerThreadCompat,
            SyscallGroup::SignalCompat,
            SyscallGroup::CompilerExec,
            SyscallGroup::CompilerExtras,
        ],
    }
}

fn seccomp_profile_syscalls(
    profile: SeccompProfile,
    syscall_profile: RuntimeSyscallProfile,
) -> Vec<&'static str> {
    let mut allowed = Vec::new();
    for group in seccomp_profile_groups(profile) {
        allowed.extend(syscall_group_expansion(*group, syscall_profile));
    }
    allowed.sort();
    allowed.dedup();
    allowed
}

fn syscall_group_name(group: SyscallGroup) -> &'static str {
    match group {
        SyscallGroup::RuntimeCore => "runtime_core",
        SyscallGroup::RuntimeClock => "runtime_clock",
        SyscallGroup::CompilerClock => "compiler_clock",
        SyscallGroup::RuntimePositionalRead => "runtime_positional_read",
        SyscallGroup::CompilerPositionalRead => "compiler_positional_read",
        SyscallGroup::RuntimeVectorIo => "runtime_vector_io",
        SyscallGroup::CompilerVectorIo => "compiler_vector_io",
        SyscallGroup::RuntimeFileStatCompat => "runtime_file_stat_compat",
        SyscallGroup::CompilerFileStatCompat => "compiler_file_stat_compat",
        SyscallGroup::FileOpenCompat => "file_open_compat",
        SyscallGroup::RuntimeProcessLifecycleCompat => "runtime_process_lifecycle_compat",
        SyscallGroup::CompilerProcessLifecycleCompat => "compiler_process_lifecycle_compat",
        SyscallGroup::RuntimeThreadCompat => "runtime_thread_compat",
        SyscallGroup::CompilerThreadCompat => "compiler_thread_compat",
        SyscallGroup::SignalCompat => "signal_compat",
        SyscallGroup::CppRuntimeExtras => "cpp_runtime_extras",
        SyscallGroup::RustRuntimeExtras => "rust_runtime_extras",
        SyscallGroup::PythonRuntimeExtras => "python_runtime_extras",
        SyscallGroup::WasmtimeRuntimeExtras => "wasmtime_runtime_extras",
        SyscallGroup::CompilerExec => "compiler_exec",
        SyscallGroup::CompilerExtras => "compiler_extras",
    }
}

fn syscall_group_expansion(
    group: SyscallGroup,
    syscall_profile: RuntimeSyscallProfile,
) -> Vec<&'static str> {
    let flavor = syscall_profile.flavor;
    let arch = syscall_profile.arch;
    let mut allowed = match group {
        SyscallGroup::RuntimeCore => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec![
                "access",
                "arch_prctl",
                "brk",
                "close",
                "exit_group",
                "faccessat",
                "futex",
                "getrandom",
                "lseek",
                "mmap",
                "mprotect",
                "munmap",
                "prlimit64",
                "read",
                "readlink",
                "rseq",
                "set_robust_list",
                "set_tid_address",
                "write",
            ],
            RuntimeSyscallFlavor::Arch => vec![
                "access",
                "arch_prctl",
                "brk",
                "close",
                "exit_group",
                "faccessat",
                "futex",
                "getrandom",
                "lseek",
                "mmap",
                "mprotect",
                "munmap",
                "prlimit64",
                "read",
                "readlink",
                "rseq",
                "set_robust_list",
                "set_tid_address",
                "write",
            ],
            RuntimeSyscallFlavor::RhelLike => vec![
                "access",
                "arch_prctl",
                "brk",
                "close",
                "exit_group",
                "faccessat",
                "futex",
                "getrandom",
                "lseek",
                "mmap",
                "mprotect",
                "munmap",
                "prlimit64",
                "read",
                "readlink",
                "rseq",
                "set_robust_list",
                "set_tid_address",
                "write",
            ],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec![
                "access",
                "arch_prctl",
                "brk",
                "close",
                "exit_group",
                "faccessat",
                "futex",
                "getrandom",
                "lseek",
                "mmap",
                "mprotect",
                "munmap",
                "prlimit64",
                "read",
                "readlink",
                "rseq",
                "set_robust_list",
                "set_tid_address",
                "write",
            ],
        },
        SyscallGroup::RuntimeClock => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => Vec::new(),
            RuntimeSyscallFlavor::Arch => vec!["clock_gettime"],
            RuntimeSyscallFlavor::RhelLike => vec!["clock_gettime"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["clock_gettime"],
        },
        SyscallGroup::CompilerClock => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["clock_gettime"],
            RuntimeSyscallFlavor::Arch => vec!["clock_gettime"],
            RuntimeSyscallFlavor::RhelLike => vec!["clock_gettime"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["clock_gettime"],
        },
        SyscallGroup::RuntimePositionalRead => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => Vec::new(),
            RuntimeSyscallFlavor::Arch => vec!["pread64"],
            RuntimeSyscallFlavor::RhelLike => Vec::new(),
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["pread64"],
        },
        SyscallGroup::CompilerPositionalRead => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["pread64"],
            RuntimeSyscallFlavor::Arch => vec!["pread64"],
            RuntimeSyscallFlavor::RhelLike => Vec::new(),
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["pread64"],
        },
        SyscallGroup::RuntimeVectorIo => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => Vec::new(),
            RuntimeSyscallFlavor::Arch => vec!["readv", "writev"],
            RuntimeSyscallFlavor::RhelLike => Vec::new(),
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["readv", "writev"],
        },
        SyscallGroup::CompilerVectorIo => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["readv", "writev"],
            RuntimeSyscallFlavor::Arch => vec!["readv", "writev"],
            RuntimeSyscallFlavor::RhelLike => Vec::new(),
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["readv", "writev"],
        },
        SyscallGroup::RuntimeFileStatCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["fstat", "newfstatat", "statx"],
            RuntimeSyscallFlavor::Arch => vec!["fstat", "newfstatat", "statx"],
            RuntimeSyscallFlavor::RhelLike => vec!["newfstat", "newfstatat", "statx"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => {
                vec!["newfstat", "newfstatat"]
            }
        },
        SyscallGroup::CompilerFileStatCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["fstat", "newfstat", "newfstatat", "statx"],
            RuntimeSyscallFlavor::Arch => vec!["fstat", "newfstatat", "statx"],
            RuntimeSyscallFlavor::RhelLike => vec!["newfstat", "newfstatat", "statx"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => {
                vec!["newfstat", "newfstatat"]
            }
        },
        SyscallGroup::FileOpenCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => {
                vec!["open", "openat", "readlink", "readlinkat", "unlink"]
            }
            RuntimeSyscallFlavor::Arch => vec!["open", "openat", "readlinkat", "unlink"],
            RuntimeSyscallFlavor::RhelLike => vec!["openat", "readlinkat", "unlink"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => {
                vec!["open", "openat", "readlinkat", "unlink"]
            }
        },
        SyscallGroup::RuntimeProcessLifecycleCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => Vec::new(),
            RuntimeSyscallFlavor::Arch => vec!["wait4"],
            RuntimeSyscallFlavor::RhelLike => vec!["wait4", "waitid"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["wait4"],
        },
        SyscallGroup::CompilerProcessLifecycleCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["wait4", "waitid"],
            RuntimeSyscallFlavor::Arch => vec!["wait4"],
            RuntimeSyscallFlavor::RhelLike => vec!["wait4", "waitid"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["wait4"],
        },
        SyscallGroup::RuntimeThreadCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["clone"],
            RuntimeSyscallFlavor::Arch => vec!["clone", "clone3"],
            RuntimeSyscallFlavor::RhelLike => vec!["clone3"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["clone", "clone3"],
        },
        SyscallGroup::CompilerThreadCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["clone", "clone3"],
            RuntimeSyscallFlavor::Arch => vec!["clone", "clone3"],
            RuntimeSyscallFlavor::RhelLike => vec!["clone3"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["clone", "clone3"],
        },
        SyscallGroup::SignalCompat => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec![
                "rt_sigaction",
                "rt_sigprocmask",
                "sigaltstack",
                "rt_sigreturn",
            ],
            RuntimeSyscallFlavor::Arch => vec!["rt_sigaction", "rt_sigprocmask", "sigaltstack"],
            RuntimeSyscallFlavor::RhelLike => vec!["rt_sigaction", "rt_sigprocmask"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => {
                vec!["rt_sigaction", "rt_sigprocmask", "sigaltstack"]
            }
        },
        SyscallGroup::CppRuntimeExtras => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["ioctl"],
            RuntimeSyscallFlavor::Arch => vec!["ioctl"],
            RuntimeSyscallFlavor::RhelLike => Vec::new(),
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["ioctl"],
        },
        SyscallGroup::RustRuntimeExtras => {
            let mut syscalls = match flavor {
                RuntimeSyscallFlavor::DebianUbuntu => {
                    vec!["gettid", "poll", "sched_getaffinity"]
                }
                RuntimeSyscallFlavor::Arch => vec!["gettid", "poll", "sched_getaffinity"],
                RuntimeSyscallFlavor::RhelLike => vec!["gettid", "poll"],
                RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => {
                    vec!["gettid", "poll", "sched_getaffinity"]
                }
            };
            if arch == RuntimeSyscallArch::Aarch64 {
                syscalls.push("ppoll");
            }
            syscalls
        }
        SyscallGroup::PythonRuntimeExtras => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => {
                let mut syscalls = vec![
                    "dup",
                    "dup2",
                    "getcwd",
                    "getdents64",
                    "getegid",
                    "geteuid",
                    "getgid",
                    "gettid",
                    "getuid",
                    "pipe2",
                    "readlink",
                    "sysinfo",
                ];
                if arch == RuntimeSyscallArch::Aarch64 {
                    syscalls.retain(|&s| !matches!(s, "getdents64" | "pipe2"));
                }
                syscalls
            }
            RuntimeSyscallFlavor::Arch => vec![
                "dup",
                "dup2",
                "getcwd",
                "getdents64",
                "getegid",
                "geteuid",
                "getgid",
                "gettid",
                "getuid",
                "pipe2",
                "sysinfo",
            ],
            RuntimeSyscallFlavor::RhelLike => vec![
                "dup",
                "dup2",
                "getcwd",
                "getdents64",
                "getegid",
                "geteuid",
                "getgid",
                "gettid",
                "getuid",
                "pipe2",
            ],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec![
                "dup",
                "dup2",
                "getcwd",
                "getdents64",
                "getegid",
                "geteuid",
                "getgid",
                "gettid",
                "getuid",
                "pipe2",
                "sysinfo",
            ],
        },
        SyscallGroup::WasmtimeRuntimeExtras => {
            let mut syscalls = match flavor {
                RuntimeSyscallFlavor::DebianUbuntu => vec![
                    "clock_nanosleep",
                    "dup",
                    "dup2",
                    "epoll_create1",
                    "epoll_ctl",
                    "epoll_wait",
                    "eventfd2",
                    "exit",
                    "fcntl",
                    "getdents64",
                    "getpriority",
                    "gettid",
                    "ioctl",
                    "madvise",
                    "memfd_create",
                    "mkdir",
                    "pipe2",
                    "poll",
                    "prctl",
                    "rename",
                    "sched_getaffinity",
                    "sched_yield",
                    "setpriority",
                    "socketpair",
                    "sysinfo",
                ],
                RuntimeSyscallFlavor::Arch => vec![
                    "clock_nanosleep",
                    "dup",
                    "dup2",
                    "epoll_create1",
                    "epoll_ctl",
                    "epoll_wait",
                    "eventfd2",
                    "exit",
                    "fcntl",
                    "getdents64",
                    "getpriority",
                    "gettid",
                    "ioctl",
                    "madvise",
                    "memfd_create",
                    "mkdir",
                    "pipe2",
                    "poll",
                    "prctl",
                    "rename",
                    "sched_getaffinity",
                    "sched_yield",
                    "setpriority",
                    "socketpair",
                    "sysinfo",
                ],
                RuntimeSyscallFlavor::RhelLike => vec![
                    "clock_nanosleep",
                    "dup",
                    "dup2",
                    "epoll_create1",
                    "epoll_ctl",
                    "epoll_wait",
                    "eventfd2",
                    "exit",
                    "fcntl",
                    "getdents64",
                    "getpriority",
                    "gettid",
                    "ioctl",
                    "madvise",
                    "mkdir",
                    "pipe2",
                    "poll",
                    "prctl",
                    "rename",
                    "sched_getaffinity",
                    "setpriority",
                    "socketpair",
                    "sysinfo",
                ],
                RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec![
                    "clock_nanosleep",
                    "dup",
                    "dup2",
                    "epoll_create1",
                    "epoll_ctl",
                    "epoll_wait",
                    "eventfd2",
                    "exit",
                    "fcntl",
                    "getdents64",
                    "getpriority",
                    "gettid",
                    "ioctl",
                    "madvise",
                    "memfd_create",
                    "mkdir",
                    "pipe2",
                    "poll",
                    "prctl",
                    "rename",
                    "sched_getaffinity",
                    "sched_yield",
                    "setpriority",
                    "socketpair",
                    "sysinfo",
                ],
            };
            if matches!(flavor, RuntimeSyscallFlavor::DebianUbuntu)
                && arch == RuntimeSyscallArch::Aarch64
            {
                syscalls.retain(|&s| {
                    !matches!(
                        s,
                        "dup" | "getdents64" | "getpid" | "pipe2" | "sched_yield" | "sysinfo"
                    )
                });
            }
            if arch == RuntimeSyscallArch::Aarch64 {
                syscalls.extend([
                    "epoll_pwait",
                    "getpid",
                    "membarrier",
                    "mkdirat",
                    "mremap",
                    "ppoll",
                    "renameat",
                ]);
            }
            if matches!(flavor, RuntimeSyscallFlavor::DebianUbuntu)
                && arch == RuntimeSyscallArch::Aarch64
            {
                syscalls.retain(|&s| s != "getpid");
            }
            syscalls
        }
        SyscallGroup::CompilerExec => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec!["execve", "execveat"],
            RuntimeSyscallFlavor::Arch => vec!["execve", "execveat"],
            RuntimeSyscallFlavor::RhelLike => vec!["execve"],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec!["execve"],
        },
        SyscallGroup::CompilerExtras => match flavor {
            RuntimeSyscallFlavor::DebianUbuntu => vec![
                "dup", "dup2", "getcwd", "getegid", "geteuid", "getgid", "gettid", "getuid",
                "ioctl", "pipe2", "sysinfo",
            ],
            RuntimeSyscallFlavor::Arch => vec![
                "dup", "dup2", "getcwd", "getegid", "geteuid", "getgid", "gettid", "getuid",
                "ioctl", "pipe2", "sysinfo",
            ],
            RuntimeSyscallFlavor::RhelLike => vec![
                "dup", "dup2", "getcwd", "getegid", "geteuid", "getgid", "gettid", "getuid",
                "ioctl", "pipe2",
            ],
            RuntimeSyscallFlavor::Generic | RuntimeSyscallFlavor::Auto => vec![
                "dup", "dup2", "getcwd", "getegid", "geteuid", "getgid", "gettid", "getuid",
                "ioctl", "pipe2", "sysinfo",
            ],
        },
    };

    if arch == RuntimeSyscallArch::Aarch64 {
        allowed.retain(|&s| {
            !matches!(
                s,
                "access"
                    | "arch_prctl"
                    | "dup2"
                    | "epoll_wait"
                    | "fstat"
                    | "mkdir"
                    | "open"
                    | "poll"
                    | "readlink"
                    | "rename"
                    | "unlink"
            )
        });
    }

    allowed
}

fn normalize_syscall_name(syscall: &'static str) -> &'static str {
    match syscall {
        "fstat" | "newfstat" | "newfstatat" | "statx" => "file_stat",
        "clone" | "clone3" | "vfork" => "process_clone",
        "rt_sigaction" | "rt_sigprocmask" | "sigaltstack" | "rt_sigreturn" => "signal_runtime",
        "sched_getaffinity" | "sched_yield" => "scheduler_affinity",
        other => other,
    }
}

pub fn debug_seccomp_profile_group_names(policy: &str) -> Vec<&'static str> {
    let mut names = seccomp_profile_groups(seccomp_profile_for_policy(policy))
        .iter()
        .map(|group| syscall_group_name(*group))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

pub fn debug_seccomp_profile_syscalls(policy: &str) -> Vec<&'static str> {
    debug_seccomp_profile_syscalls_for_target(
        policy,
        RuntimeSyscallFlavor::Auto,
        RuntimeSyscallArch::Auto,
    )
}

pub fn debug_seccomp_profile_normalized_syscalls(policy: &str) -> Vec<&'static str> {
    debug_seccomp_profile_normalized_syscalls_for_target(
        policy,
        RuntimeSyscallFlavor::Auto,
        RuntimeSyscallArch::Auto,
    )
}

pub fn debug_seccomp_profile_syscalls_for_flavor(
    policy: &str,
    flavor: RuntimeSyscallFlavor,
) -> Vec<&'static str> {
    debug_seccomp_profile_syscalls_for_target(policy, flavor, RuntimeSyscallArch::Auto)
}

pub fn debug_seccomp_profile_syscalls_for_target(
    policy: &str,
    flavor: RuntimeSyscallFlavor,
    arch: RuntimeSyscallArch,
) -> Vec<&'static str> {
    seccomp_profile_syscalls(
        seccomp_profile_for_policy(policy),
        resolve_runtime_syscall_profile(flavor, arch),
    )
}

pub fn debug_seccomp_profile_normalized_syscalls_for_flavor(
    policy: &str,
    flavor: RuntimeSyscallFlavor,
) -> Vec<&'static str> {
    debug_seccomp_profile_normalized_syscalls_for_target(policy, flavor, RuntimeSyscallArch::Auto)
}

pub fn debug_seccomp_profile_normalized_syscalls_for_target(
    policy: &str,
    flavor: RuntimeSyscallFlavor,
    arch: RuntimeSyscallArch,
) -> Vec<&'static str> {
    let mut normalized = debug_seccomp_profile_syscalls_for_target(policy, flavor, arch)
        .into_iter()
        .map(normalize_syscall_name)
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub fn debug_detected_runtime_syscall_flavor() -> RuntimeSyscallFlavor {
    debug_detected_runtime_syscall_profile().flavor
}

pub fn debug_detected_runtime_syscall_arch() -> RuntimeSyscallArch {
    debug_detected_runtime_syscall_profile().arch
}

pub fn debug_detected_runtime_syscall_profile() -> RuntimeSyscallProfile {
    resolve_runtime_syscall_profile(RuntimeSyscallFlavor::Auto, RuntimeSyscallArch::Auto)
}

fn resolve_runtime_syscall_flavor(flavor: RuntimeSyscallFlavor) -> RuntimeSyscallFlavor {
    match flavor {
        RuntimeSyscallFlavor::Auto => detect_runtime_syscall_flavor(),
        other => other,
    }
}

fn resolve_runtime_syscall_arch(arch: RuntimeSyscallArch) -> RuntimeSyscallArch {
    match arch {
        RuntimeSyscallArch::Auto => detect_runtime_syscall_arch(),
        other => other,
    }
}

fn resolve_runtime_syscall_profile(
    flavor: RuntimeSyscallFlavor,
    arch: RuntimeSyscallArch,
) -> RuntimeSyscallProfile {
    RuntimeSyscallProfile {
        flavor: resolve_runtime_syscall_flavor(flavor),
        arch: resolve_runtime_syscall_arch(arch),
    }
}

fn detect_runtime_syscall_flavor() -> RuntimeSyscallFlavor {
    let Ok(os_release) = fs::read_to_string("/etc/os-release") else {
        return RuntimeSyscallFlavor::Generic;
    };
    let lower = os_release.to_lowercase();
    if lower.contains("id=ubuntu")
        || lower.contains("id=debian")
        || lower.contains("id_like=ubuntu")
        || lower.contains("id_like=debian")
    {
        return RuntimeSyscallFlavor::DebianUbuntu;
    }
    if lower.contains("id=arch") || lower.contains("id=manjaro") || lower.contains("id_like=arch") {
        return RuntimeSyscallFlavor::Arch;
    }
    if lower.contains("id=fedora")
        || lower.contains("id=centos")
        || lower.contains("id=rhel")
        || lower.contains("id=rocky")
        || lower.contains("id=almalinux")
        || lower.contains("id_like=\"rhel fedora\"")
        || lower.contains("id_like=rhel")
        || lower.contains("id_like=fedora")
    {
        return RuntimeSyscallFlavor::RhelLike;
    }
    RuntimeSyscallFlavor::Generic
}

fn detect_runtime_syscall_arch() -> RuntimeSyscallArch {
    match std::env::consts::ARCH {
        "x86_64" => RuntimeSyscallArch::X86_64,
        "aarch64" => RuntimeSyscallArch::Aarch64,
        _ => RuntimeSyscallArch::Other,
    }
}

#[cfg(test)]
fn syscall_group_aliases(group: SyscallGroup) -> Vec<&'static str> {
    match group {
        SyscallGroup::RuntimeFileStatCompat | SyscallGroup::CompilerFileStatCompat => {
            vec!["fstat", "newfstat", "newfstatat", "statx"]
        }
        _ => syscall_group_expansion(
            group,
            RuntimeSyscallProfile {
                flavor: RuntimeSyscallFlavor::Generic,
                arch: RuntimeSyscallArch::X86_64,
            },
        ),
    }
}

struct SpjExecutionSpec {
    language: String,
    source_filename: String,
    compile_command: Option<Vec<String>>,
    run_command: Vec<String>,
    readonly_mounts: Vec<String>,
}

fn build_spj_execution_spec(spj: &RuntimeSpjConfig) -> AppResult<SpjExecutionSpec> {
    match spj.language.as_str() {
        "python" | "python3" => Ok(SpjExecutionSpec {
            language: "python".to_owned(),
            source_filename: "spj.py".to_owned(),
            compile_command: None,
            run_command: vec![resolve_python_runtime(), "spj.py".to_owned()],
            readonly_mounts: Vec::new(),
        }),
        "rust" => {
            let rustc = resolve_rustc_binary();
            let readonly_mounts = rustc_mounts(&rustc);
            Ok(SpjExecutionSpec {
                language: "rust".to_owned(),
                source_filename: "spj.rs".to_owned(),
                compile_command: Some(vec![
                    rustc,
                    "-O".to_owned(),
                    "-C".to_owned(),
                    "linker=/usr/bin/cc".to_owned(),
                    "-C".to_owned(),
                    "link-arg=-fuse-ld=bfd".to_owned(),
                    "-o".to_owned(),
                    "spj.exe".to_owned(),
                    "spj.rs".to_owned(),
                ]),
                run_command: vec!["./spj.exe".to_owned()],
                readonly_mounts,
            })
        }
        "cpp" | "c++" => Ok(SpjExecutionSpec {
            language: "cpp".to_owned(),
            source_filename: "spj.cpp".to_owned(),
            compile_command: Some(vec![
                resolve_native_cpp_compiler(),
                "-std=c++20".to_owned(),
                "-O2".to_owned(),
                "-pipe".to_owned(),
                "-o".to_owned(),
                "spj.exe".to_owned(),
                "spj.cpp".to_owned(),
            ]),
            run_command: vec!["./spj.exe".to_owned()],
            readonly_mounts: Vec::new(),
        }),
        other => Err(AppError::BadRequest(format!(
            "unsupported spj language: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compiler_seccomp_policy_string, default_runtime_worker_groups,
        seccomp_policy_string_with_mode, seccomp_policy_string_with_mode_and_arch,
        InMemoryRuntimeTaskQueue, RuntimeNodeHealthStatus, RuntimeRouteBinding, RuntimeSeccompMode,
        RuntimeSyscallArch, RuntimeSyscallProfile, RuntimeTaskQueue, RuntimeTaskService,
        RuntimeWorkerGroup,
    };
    use crate::planning::{
        CppRuntimeSpec, LanguageRuntimeSpec, PythonRuntimeSpec, RuntimeExecutionBackend,
        RuntimeLanguageCatalog, RustRuntimeSpec,
    };
    use crate::{
        OjJudgeTask, RuntimeJudgeMode, RuntimeLimits, RuntimeRetryPolicy, RuntimeSandboxKind,
        RuntimeStageStatus, RuntimeSyscallFlavor, RuntimeTask, RuntimeTaskLifecycleStatus,
        RuntimeTaskPayload, RuntimeTaskType, RuntimeTestcase, RuntimeWorker,
    };
    use nexus_shared::{ProblemId, SubmissionId, UserId};
    use std::{
        fs,
        path::Path,
        process::Command as StdCommand,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[tokio::test]
    async fn route_aware_queue_keeps_fifo_within_same_route() {
        let queue = InMemoryRuntimeTaskQueue::default();
        queue
            .enqueue(runtime_task("task-1", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");
        queue
            .enqueue(runtime_task("task-2", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");

        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-1".to_owned())
        );
        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-2".to_owned())
        );
    }

    #[tokio::test]
    async fn route_aware_queue_round_robins_across_routes() {
        let queue = InMemoryRuntimeTaskQueue::default();
        queue
            .enqueue(runtime_task("task-1", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");
        queue
            .enqueue(runtime_task("task-2", "oj_judge", "special"))
            .await
            .expect("enqueue should succeed");
        queue
            .enqueue(runtime_task("task-3", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");

        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-1".to_owned())
        );
        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-2".to_owned())
        );
        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-3".to_owned())
        );
    }

    #[tokio::test]
    async fn route_aware_queue_respects_worker_group_bindings() {
        let queue = InMemoryRuntimeTaskQueue::default();
        queue
            .enqueue(runtime_task("task-1", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");
        queue
            .enqueue(runtime_task("task-2", "oj_judge", "special"))
            .await
            .expect("enqueue should succeed");

        let bindings = vec![RuntimeRouteBinding {
            queue: "oj_judge".to_owned(),
            lane: "special".to_owned(),
        }];

        assert_eq!(
            queue
                .reserve(&bindings)
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-2".to_owned())
        );
        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|delivery| delivery.task.task_id),
            Some("task-1".to_owned())
        );
    }

    #[test]
    fn default_worker_groups_cover_all_oj_lanes() {
        let worker_groups = default_runtime_worker_groups();
        let mut routes = worker_groups
            .into_iter()
            .flat_map(|group| group.bindings.into_iter())
            .map(|binding| (binding.queue, binding.lane))
            .collect::<Vec<_>>();
        routes.sort();

        assert_eq!(
            routes,
            vec![
                ("oj_judge".to_owned(), "fast".to_owned()),
                ("oj_judge".to_owned(), "heavy".to_owned()),
                ("oj_judge".to_owned(), "normal".to_owned()),
                ("oj_judge".to_owned(), "special".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn runtime_task_service_exposes_started_worker_groups() {
        let nsjail = detect_nsjail_binary().unwrap_or_else(|| "/usr/bin/nsjail".to_owned());
        let service = RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                RuntimeLanguageCatalog::default(),
                "/tmp/nexuscode-runtime-test",
                nsjail,
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                current_test_arch(),
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(super::NoopRuntimeEventObserver),
        );

        let groups = vec![RuntimeWorkerGroup {
            name: "oj-fast".to_owned(),
            bindings: vec![RuntimeRouteBinding {
                queue: "oj_judge".to_owned(),
                lane: "fast".to_owned(),
            }],
        }];
        service.start_background_workers(groups.clone());

        assert_eq!(service.worker_groups(), groups);
    }

    #[test]
    fn runtime_task_service_exposes_registered_node_status() {
        let nsjail = detect_nsjail_binary().unwrap_or_else(|| "/usr/bin/nsjail".to_owned());
        let service = RuntimeTaskService::with_queue(
            Arc::new(RuntimeWorker::new(
                RuntimeLanguageCatalog::default(),
                "/tmp/nexuscode-runtime-test",
                nsjail,
                RuntimeSeccompMode::Log,
                RuntimeSyscallFlavor::Generic,
                current_test_arch(),
            )),
            Arc::new(InMemoryRuntimeTaskQueue::default()),
            Arc::new(super::NoopRuntimeEventObserver),
        );
        service.register_node("runtime-node-a");

        let status = service.node_status();
        assert_eq!(status.node_id, "runtime-node-a");
        assert!(status.started_at_ms > 0);
        assert!(status.last_heartbeat_ms >= status.started_at_ms);
        assert_eq!(status.node_status, RuntimeNodeHealthStatus::Healthy);
        assert!(status.worker_groups.is_empty());
    }

    #[tokio::test]
    async fn queue_retry_requeues_message_with_incremented_attempt() {
        let queue = InMemoryRuntimeTaskQueue::default();
        queue
            .enqueue(runtime_task("task-1", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");

        let delivery = queue.reserve(&[]).await.expect("delivery should exist");
        let disposition = queue
            .retry(&delivery.delivery_id, "worker crash", 0)
            .await
            .expect("retry should succeed");

        assert_eq!(disposition, super::RetryDisposition::Requeued);
        let retried = queue
            .reserve(&[])
            .await
            .expect("retried delivery should exist");
        assert_eq!(retried.task.task_id, "task-1");
        assert_eq!(retried.attempt, 2);
        assert_eq!(retried.last_error.as_deref(), Some("worker crash"));
    }

    #[tokio::test]
    async fn queue_retry_promotes_to_dead_letter_after_max_attempts() {
        let queue = InMemoryRuntimeTaskQueue::default();
        let mut task = runtime_task("task-1", "oj_judge", "fast");
        task.retry_policy.max_attempts = 1;
        queue.enqueue(task).await.expect("enqueue should succeed");

        let delivery = queue.reserve(&[]).await.expect("delivery should exist");
        let disposition = queue
            .retry(&delivery.delivery_id, "worker crash", 0)
            .await
            .expect("retry should succeed");

        assert_eq!(disposition, super::RetryDisposition::DeadLettered);
        let dead_letters = queue
            .dead_letters()
            .await
            .expect("dead letters should be readable");
        assert_eq!(dead_letters.len(), 1);
        assert_eq!(dead_letters[0].task_id, "task-1");
        assert!(queue.reserve(&[]).await.is_none());
    }

    #[tokio::test]
    async fn queue_stats_include_queued_leased_and_dead_lettered_counts() {
        let queue = InMemoryRuntimeTaskQueue::default();
        let mut dead_letter_task = runtime_task("task-1", "oj_judge", "fast");
        dead_letter_task.retry_policy.max_attempts = 1;
        queue
            .enqueue(dead_letter_task)
            .await
            .expect("enqueue should succeed");
        queue
            .enqueue(runtime_task("task-2", "oj_judge", "fast"))
            .await
            .expect("enqueue should succeed");

        let first = queue.reserve(&[]).await.expect("delivery should exist");
        let second = queue.reserve(&[]).await.expect("delivery should exist");
        let disposition = queue
            .retry(&first.delivery_id, "worker crash", 0)
            .await
            .expect("retry should succeed");
        assert_eq!(disposition, super::RetryDisposition::DeadLettered);

        let stats = queue.stats().await.expect("stats should be readable");
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].queue, "oj_judge");
        assert_eq!(stats[0].lane, "fast");
        assert_eq!(stats[0].queued, 0);
        assert_eq!(stats[0].leased, 1);
        assert_eq!(stats[0].dead_lettered, 1);

        queue
            .ack(&second.delivery_id)
            .await
            .expect("ack should succeed");
    }

    #[tokio::test]
    async fn dead_letter_can_be_replayed_back_to_queue() {
        let queue = InMemoryRuntimeTaskQueue::default();
        let mut task = runtime_task("task-1", "oj_judge", "fast");
        task.retry_policy.max_attempts = 1;
        queue.enqueue(task).await.expect("enqueue should succeed");

        let delivery = queue.reserve(&[]).await.expect("delivery should exist");
        let disposition = queue
            .retry(&delivery.delivery_id, "worker crash", 0)
            .await
            .expect("retry should succeed");
        assert_eq!(disposition, super::RetryDisposition::DeadLettered);

        let dead_letters = queue
            .dead_letters()
            .await
            .expect("dead letters should be readable");
        assert_eq!(dead_letters.len(), 1);
        let replay_receipt = queue
            .replay_dead_letter(&dead_letters[0].delivery_id)
            .await
            .expect("replay should succeed");

        assert_eq!(replay_receipt.task_id, "task-1");
        assert_eq!(replay_receipt.queue, "oj_judge");
        assert_eq!(replay_receipt.lane, "fast");
        assert!(queue
            .dead_letters()
            .await
            .expect("dead letters should be readable")
            .is_empty());
        assert_eq!(
            queue
                .reserve(&[])
                .await
                .map(|reserved| reserved.task.task_id),
            Some("task-1".to_owned())
        );

        let stats = queue.stats().await.expect("stats should be readable");
        assert_eq!(stats[0].dead_lettered, 0);
    }

    #[test]
    fn rust_runtime_spec_uses_wasm_backend_when_requested() {
        let task = wasm_runtime_task("rust");
        let plan = RustRuntimeSpec::default()
            .build_plan(&task)
            .expect("rust wasm plan should be built");

        assert_eq!(plan.execution_backend, RuntimeExecutionBackend::WasmWasi);
        assert_eq!(plan.sandbox_profile, "wasm");
        assert_eq!(plan.executable_filename.as_deref(), Some("main.wasm"));
        assert_eq!(plan.run_command[1], "run");
        assert_eq!(
            plan.run_command.last().map(String::as_str),
            Some("main.wasm")
        );
    }

    #[test]
    fn cpp_runtime_spec_uses_wasm_backend_when_requested() {
        let task = wasm_runtime_task("cpp");
        let plan = CppRuntimeSpec::default()
            .build_plan(&task)
            .expect("cpp wasm plan should be built");

        assert_eq!(plan.execution_backend, RuntimeExecutionBackend::WasmWasi);
        assert_eq!(plan.sandbox_profile, "wasm");
        assert_eq!(plan.executable_filename.as_deref(), Some("main.wasm"));
    }

    #[test]
    fn rust_runtime_spec_uses_nsjail_wasm_backend_when_requested() {
        let task = nsjail_wasm_runtime_task("rust");
        let plan = RustRuntimeSpec::default()
            .build_plan(&task)
            .expect("rust nsjail_wasm plan should be built");

        assert_eq!(plan.execution_backend, RuntimeExecutionBackend::NsjailWasm);
        assert_eq!(plan.sandbox_profile, "nsjail_wasm");
        assert_eq!(plan.executable_filename.as_deref(), Some("main.wasm"));
    }

    #[test]
    fn cpp_runtime_spec_uses_nsjail_wasm_backend_when_requested() {
        let task = nsjail_wasm_runtime_task("cpp");
        let plan = CppRuntimeSpec::default()
            .build_plan(&task)
            .expect("cpp nsjail_wasm plan should be built");

        assert_eq!(plan.execution_backend, RuntimeExecutionBackend::NsjailWasm);
        assert_eq!(plan.sandbox_profile, "nsjail_wasm");
        assert_eq!(plan.executable_filename.as_deref(), Some("main.wasm"));
    }

    #[test]
    fn python_runtime_spec_rejects_wasm() {
        let task = wasm_runtime_task("python");
        let error = PythonRuntimeSpec::default()
            .build_plan(&task)
            .expect_err("python wasm should be rejected");

        assert!(error
            .to_string()
            .contains("python does not support wasm sandbox"));
    }

    #[tokio::test]
    async fn rust_wasm_submission_executes_end_to_end_when_toolchain_is_available() {
        if !rust_wasm_toolchain_available() {
            eprintln!("skipping rust wasm smoke test: missing wasmtime or wasm32-wasip1 target");
            return;
        }
        let nsjail = detect_nsjail_binary().unwrap_or_else(|| "/usr/bin/nsjail".to_owned());

        let worker = RuntimeWorker::new(
            crate::build_default_runtime_catalog(),
            unique_test_workdir("wasm-smoke"),
            nsjail,
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
            current_test_arch(),
        );
        let task = RuntimeTask {
            task_id: "task-rust-wasm-smoke".to_owned(),
            task_type: RuntimeTaskType::OjJudge,
            source_domain: "oj".to_owned(),
            source_entity_id: "sub-rust-wasm-smoke".to_owned(),
            queue: "oj_judge".to_owned(),
            lane: "fast".to_owned(),
            retry_policy: RuntimeRetryPolicy {
                max_attempts: 1,
                retry_delay_ms: 0,
            },
            payload: RuntimeTaskPayload::OjJudge(OjJudgeTask {
                submission_id: SubmissionId::from("sub-rust-wasm-smoke"),
                problem_id: ProblemId::from("p-rust-wasm-smoke"),
                user_id: UserId::from("u-rust-wasm-smoke"),
                language: "rust".to_owned(),
                judge_mode: RuntimeJudgeMode::Acm,
                sandbox_kind: RuntimeSandboxKind::Wasm,
                source_code: r#"
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let nums: Vec<i32> = input
        .split_whitespace()
        .map(|item| item.parse::<i32>().unwrap())
        .collect();
    println!("{}", nums[0] + nums[1]);
}
"#
                .trim()
                .to_owned(),
                limits: RuntimeLimits {
                    time_limit_ms: 1000,
                    memory_limit_kb: 262144,
                },
                testcases: vec![RuntimeTestcase {
                    case_no: 1,
                    input: "1 2\n".to_owned(),
                    expected_output: "3\n".to_owned(),
                    score: 100,
                }],
                judge_config: None,
            }),
        };

        let artifacts = worker.prepare(task).expect("wasm artifacts should prepare");
        let outcome = worker
            .execute(&artifacts)
            .await
            .expect("wasm execution should succeed");

        assert!(matches!(
            outcome.final_status,
            RuntimeTaskLifecycleStatus::Completed
        ));
        assert!(outcome.compile.is_some());
        assert_eq!(outcome.cases.len(), 1, "{outcome:#?}");
        assert!(matches!(
            outcome.cases[0].status,
            super::RuntimeCaseFinalStatus::Accepted
        ));
        assert!(outcome.cases[0].stdout_excerpt.contains('3'));

        let _ = fs::remove_dir_all(&artifacts.work_dir);
    }

    #[tokio::test]
    async fn cpp_wasm_submission_executes_end_to_end_when_toolchain_is_available() {
        if !cpp_wasm_toolchain_available() {
            eprintln!("skipping cpp wasm smoke test: missing wasmtime or clang++-17");
            return;
        }
        let nsjail = detect_nsjail_binary().unwrap_or_else(|| "/usr/bin/nsjail".to_owned());

        let worker = RuntimeWorker::new(
            crate::build_default_runtime_catalog(),
            unique_test_workdir("cpp-wasm-smoke"),
            nsjail,
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
            current_test_arch(),
        );
        let task = RuntimeTask {
            task_id: "task-cpp-wasm-smoke".to_owned(),
            task_type: RuntimeTaskType::OjJudge,
            source_domain: "oj".to_owned(),
            source_entity_id: "sub-cpp-wasm-smoke".to_owned(),
            queue: "oj_judge".to_owned(),
            lane: "fast".to_owned(),
            retry_policy: RuntimeRetryPolicy {
                max_attempts: 1,
                retry_delay_ms: 0,
            },
            payload: RuntimeTaskPayload::OjJudge(OjJudgeTask {
                submission_id: SubmissionId::from("sub-cpp-wasm-smoke"),
                problem_id: ProblemId::from("p-cpp-wasm-smoke"),
                user_id: UserId::from("u-cpp-wasm-smoke"),
                language: "cpp".to_owned(),
                judge_mode: RuntimeJudgeMode::Acm,
                sandbox_kind: RuntimeSandboxKind::Wasm,
                source_code: r#"
#include <iostream>

int main() {
    int a = 0;
    int b = 0;
    if (!(std::cin >> a >> b)) {
        return 1;
    }
    std::cout << (a + b) << "\n";
    return 0;
}
"#
                .trim()
                .to_owned(),
                limits: RuntimeLimits {
                    time_limit_ms: 1000,
                    memory_limit_kb: 262144,
                },
                testcases: vec![RuntimeTestcase {
                    case_no: 1,
                    input: "1 2\n".to_owned(),
                    expected_output: "3\n".to_owned(),
                    score: 100,
                }],
                judge_config: None,
            }),
        };

        let artifacts = worker.prepare(task).expect("wasm artifacts should prepare");
        let outcome = worker
            .execute(&artifacts)
            .await
            .expect("wasm execution should succeed");

        assert!(matches!(
            outcome.final_status,
            RuntimeTaskLifecycleStatus::Completed
        ));
        assert!(outcome.compile.is_some());
        assert_eq!(outcome.cases.len(), 1);
        assert!(matches!(
            outcome.cases[0].status,
            super::RuntimeCaseFinalStatus::Accepted
        ));
        assert!(outcome.cases[0].stdout_excerpt.contains('3'));

        let _ = fs::remove_dir_all(&artifacts.work_dir);
    }

    #[tokio::test]
    async fn rust_nsjail_wasm_submission_executes_end_to_end_when_toolchain_is_available() {
        if !rust_wasm_toolchain_available() || detect_nsjail_binary().is_none() {
            eprintln!("skipping rust nsjail_wasm smoke test: missing nsjail or wasm toolchain");
            return;
        }

        let outcome = execute_runtime_task(runtime_task_with_source(
            "rust",
            RuntimeSandboxKind::NsjailWasm,
            r#"
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let nums: Vec<i32> = input
        .split_whitespace()
        .map(|item| item.parse::<i32>().unwrap())
        .collect();
    println!("{}", nums[0] + nums[1]);
}
"#,
        ))
        .await;

        assert!(
            matches!(outcome.final_status, RuntimeTaskLifecycleStatus::Completed),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::Accepted
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_wasm_submission_executes_end_to_end_when_toolchain_is_available() {
        if !cpp_wasm_toolchain_available() || detect_nsjail_binary().is_none() {
            eprintln!("skipping cpp nsjail_wasm smoke test: missing nsjail or wasm toolchain");
            return;
        }

        let outcome = execute_runtime_task(runtime_task_with_source(
            "cpp",
            RuntimeSandboxKind::NsjailWasm,
            r#"
#include <iostream>

int main() {
    int a = 0;
    int b = 0;
    if (!(std::cin >> a >> b)) {
        return 1;
    }
    std::cout << (a + b) << "\n";
    return 0;
}
"#,
        ))
        .await;

        assert!(
            matches!(outcome.final_status, RuntimeTaskLifecycleStatus::Completed),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::Accepted
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn rust_wasm_infinite_loop_is_classified_as_tle() {
        if !rust_wasm_toolchain_available() {
            eprintln!("skipping rust wasm limit test: missing wasmtime or wasm32-wasip1 target");
            return;
        }

        let outcome = execute_runtime_task(runtime_task_with_source(
            "rust",
            RuntimeSandboxKind::Wasm,
            r#"
fn main() {
    loop {}
}
"#,
        ))
        .await;

        assert!(matches!(
            outcome.final_status,
            RuntimeTaskLifecycleStatus::Failed
        ));
        assert_eq!(outcome.cases.len(), 1);
        assert!(matches!(
            outcome.cases[0].status,
            super::RuntimeCaseFinalStatus::TimeLimitExceeded
        ));
    }

    #[tokio::test]
    async fn rust_wasm_output_bomb_is_classified_as_ole() {
        if !rust_wasm_toolchain_available() {
            eprintln!("skipping rust wasm output test: missing wasmtime or wasm32-wasip1 target");
            return;
        }

        let outcome = execute_runtime_task(runtime_task_with_source_and_limits(
            "rust",
            RuntimeSandboxKind::Wasm,
            10_000,
            262_144,
            r#"
fn main() {
    loop {
        println!("0123456789012345678901234567890123456789");
    }
}
"#,
        ))
        .await;

        assert!(matches!(
            outcome.final_status,
            RuntimeTaskLifecycleStatus::Failed
        ));
        assert_eq!(outcome.cases.len(), 1);
        assert!(matches!(
            outcome.cases[0].status,
            super::RuntimeCaseFinalStatus::OutputLimitExceeded
        ));
    }

    #[tokio::test]
    async fn rust_nsjail_memory_bomb_is_classified_as_mle() {
        let outcome = execute_runtime_task(runtime_task_with_source_and_limits(
            "rust",
            RuntimeSandboxKind::Nsjail,
            1_000,
            32 * 1024,
            r#"
fn main() {
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut chunk = vec![0u8; 8 * 1024 * 1024];
        for i in (0..chunk.len()).step_by(4096) {
            chunk[i] = 1;
        }
        chunks.push(chunk);
    }
}
"#,
        ))
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::MemoryLimitExceeded
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_write_outside_workspace_is_classified_as_security_violation() {
        let outcome = execute_runtime_task(runtime_task_with_source(
            "cpp",
            RuntimeSandboxKind::Nsjail,
            r#"
#include <cerrno>
#include <cstdio>

int main() {
    FILE* file = fopen("/bin/owned", "w");
    if (file == nullptr) {
        perror("fopen");
        return 1;
    }
    fclose(file);
    return 0;
}
"#,
        ))
        .await;
        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_sleep_bypass_is_classified_as_tle() {
        let outcome = execute_runtime_task(runtime_task_with_source_and_limits(
            "cpp",
            RuntimeSandboxKind::Nsjail,
            500,
            262_144,
            r#"
#include <chrono>
#include <thread>

int main() {
    std::this_thread::sleep_for(std::chrono::seconds(10));
    return 0;
}
"#,
        ))
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::TimeLimitExceeded
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_network_probe_is_classified_as_security_violation() {
        let outcome = execute_runtime_task(runtime_task_with_source(
            "cpp",
            RuntimeSandboxKind::Nsjail,
            r#"
#include <arpa/inet.h>
#include <cerrno>
#include <cstdio>
#include <cstring>
#include <sys/socket.h>
#include <unistd.h>

int main() {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        perror("socket");
        return 1;
    }

    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(53);
    inet_pton(AF_INET, "8.8.8.8", &addr.sin_addr);

    int rc = connect(fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr));
    if (rc < 0) {
        perror("connect");
        close(fd);
        return 1;
    }

    close(fd);
    return 0;
}
"#,
        ))
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_thread_spawn_degrades_to_tle_under_log_mode() {
        let outcome = execute_runtime_task(runtime_task_with_source(
            "cpp",
            RuntimeSandboxKind::Nsjail,
            r#"
#include <thread>

int main() {
    std::thread worker([] {
        for (;;) {}
    });
    worker.join();
    return 0;
}
"#,
        ))
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::TimeLimitExceeded
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_thread_spawn_is_blocked_under_kill_mode() {
        let outcome = execute_runtime_task_with_seccomp_mode(
            runtime_task_with_source(
                "cpp",
                RuntimeSandboxKind::Nsjail,
                r#"
#include <thread>

int main() {
    std::thread worker([] {
        for (;;) {}
    });
    worker.join();
    return 0;
}
"#,
            ),
            RuntimeSeccompMode::Kill,
        )
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
                    | super::RuntimeCaseFinalStatus::RuntimeError
                    | super::RuntimeCaseFinalStatus::TimeLimitExceeded
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn cpp_nsjail_execve_bypass_via_system_is_classified_as_security_violation() {
        let outcome = execute_runtime_task_with_seccomp_mode(
            runtime_task_with_source(
                "cpp",
                RuntimeSandboxKind::Nsjail,
                r#"
#include <cstdlib>

int main() {
    return system("/bin/sh -c 'echo pwned'");
}
"#,
            ),
            RuntimeSeccompMode::Kill,
        )
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
                    | super::RuntimeCaseFinalStatus::RuntimeError
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(
            !outcome.cases[0].stdout_excerpt.contains("pwned"),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn rust_nsjail_execve_bypass_via_command_is_classified_as_security_violation() {
        let outcome = execute_runtime_task_with_seccomp_mode(
            runtime_task_with_source(
                "rust",
                RuntimeSandboxKind::Nsjail,
                r#"
use std::process::Command;

fn main() {
    let _ = Command::new("/bin/sh")
        .arg("-c")
        .arg("echo pwned")
        .status();
}
"#,
            ),
            RuntimeSeccompMode::Kill,
        )
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
                    | super::RuntimeCaseFinalStatus::RuntimeError
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(
            !outcome.cases[0].stdout_excerpt.contains("pwned"),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn rust_nsjail_include_bytes_host_file_attack_is_blocked_at_compile_time() {
        let outcome = execute_runtime_task(runtime_task_with_source(
            "rust",
            RuntimeSandboxKind::Nsjail,
            r#"
const SECRET: &[u8] = include_bytes!("/etc/passwd");

fn main() {
    println!("{}", SECRET.len());
}
"#,
        ))
        .await;

        assert!(
            matches!(outcome.final_status, RuntimeTaskLifecycleStatus::Failed),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(
            outcome.cases.is_empty(),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        let compile = outcome
            .compile
            .as_ref()
            .expect("compile outcome should exist");
        assert!(matches!(compile.status, RuntimeStageStatus::Failed));
        assert_eq!(
            compile.failure_kind,
            Some(super::RuntimeFailureKind::SecurityViolation),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(compile.stderr_excerpt.contains("/etc/passwd"));
        assert!(!compile.stderr_excerpt.contains("root:x:"));
    }

    #[tokio::test]
    async fn python_nsjail_write_outside_workspace_is_classified_as_security_violation() {
        let outcome = execute_runtime_task(runtime_task_with_source(
            "python",
            RuntimeSandboxKind::Nsjail,
            r#"
with open("/bin/owned", "w", encoding="utf-8") as handle:
    handle.write("pwned")
"#,
        ))
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn python_nsjail_network_probe_is_classified_as_security_violation() {
        let outcome = execute_runtime_task(runtime_task_with_source(
            "python",
            RuntimeSandboxKind::Nsjail,
            r#"
import socket

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    sock.connect(("8.8.8.8", 53))
finally:
    sock.close()
"#,
        ))
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[tokio::test]
    async fn python_nsjail_execve_bypass_via_subprocess_is_classified_as_security_violation() {
        let outcome = execute_runtime_task_with_seccomp_mode(
            runtime_task_with_source(
                "python",
                RuntimeSandboxKind::Nsjail,
                r#"
import subprocess

subprocess.run(["/bin/sh", "-c", "echo pwned"], check=False)
"#,
            ),
            RuntimeSeccompMode::Kill,
        )
        .await;

        assert_failed_with_single_case(&outcome);
        assert!(
            matches!(
                outcome.cases[0].status,
                super::RuntimeCaseFinalStatus::SecurityViolation
                    | super::RuntimeCaseFinalStatus::RuntimeError
            ),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
        assert!(
            !outcome.cases[0].stdout_excerpt.contains("pwned"),
            "unexpected outcome:\n{}",
            outcome_debug(&outcome)
        );
    }

    #[test]
    fn runtime_seccomp_policies_currently_allow_execve_while_compiler_needs_it() {
        let cpp_policy = seccomp_policy_string_with_mode(
            "cpp_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let rust_policy = seccomp_policy_string_with_mode(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let python_policy = seccomp_policy_string_with_mode(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let wasm_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::Generic,
            arch: RuntimeSyscallArch::X86_64,
        });

        assert!(!cpp_policy.contains("execve"));
        assert!(!rust_policy.contains("execve"));
        assert!(!python_policy.contains("execve"));
        assert!(!wasm_policy.contains("execve"));
        assert!(compiler_policy.contains("execve"));
    }

    #[test]
    fn runtime_seccomp_enforcement_mode_changes_default_action() {
        let log_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let kill_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Kill,
            RuntimeSyscallFlavor::Generic,
        );

        assert!(log_policy.contains("DEFAULT LOG"));
        assert!(kill_policy.contains("DEFAULT KILL"));
    }

    #[test]
    fn runtime_seccomp_profiles_expand_file_stat_compat_aliases() {
        let aliases = super::syscall_group_aliases(super::SyscallGroup::RuntimeFileStatCompat);
        let cpp_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let rust_policy = seccomp_policy_string_with_mode(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let wasm_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );

        for alias in ["fstat", "newfstat", "newfstatat", "statx"] {
            assert!(aliases.contains(&alias));
        }
        assert!(cpp_policy.contains("newfstat"));
        assert!(cpp_policy.contains("newfstatat"));
        assert!(rust_policy.contains("newfstat"));
        assert!(wasm_policy.contains("newfstatat"));
    }

    #[test]
    fn runtime_seccomp_profiles_are_split_by_runtime_shape() {
        let cpp_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let rust_policy = seccomp_policy_string_with_mode(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let wasm_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );

        assert!(cpp_policy.contains("ioctl"));
        assert!(!cpp_policy.contains("poll"));
        assert!(!cpp_policy.contains("clone"));
        assert!(!rust_policy.contains("clone"));
        assert!(rust_policy.contains("poll"));
        assert!(wasm_policy.contains("epoll_create1"));
        assert!(wasm_policy.contains("clone3"));
        assert!(wasm_policy.contains("memfd_create"));
        assert!(!cpp_policy.contains("memfd_create"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_file_stat_compat_by_linux_family() {
        let generic_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let debian_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
        );
        let arch_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Arch,
        );

        assert!(!generic_policy.contains("statx"));
        assert!(debian_policy.contains("fstat"));
        assert!(debian_policy.contains("statx"));
        assert!(arch_policy.contains("fstat"));
        assert!(arch_policy.contains("statx"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_signal_and_clone_groups_by_linux_family() {
        let generic_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let debian_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
        );
        let rhel_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::RhelLike,
        );

        assert!(generic_policy.contains("clone"));
        assert!(generic_policy.contains("clone3"));
        assert!(debian_policy.contains("rt_sigreturn"));
        assert!(!generic_policy.contains("rt_sigreturn"));
        assert!(rhel_policy.contains("clone3"));
        assert!(!rhel_policy.contains("clone,"));
        assert!(!rhel_policy.contains("sigaltstack"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_file_open_and_wasmtime_groups_by_linux_family() {
        let generic_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let debian_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
        );
        let rhel_policy = seccomp_policy_string_with_mode(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::RhelLike,
        );

        assert!(generic_policy.contains("open,"));
        assert!(debian_policy.contains("readlink"));
        assert!(generic_policy.contains("memfd_create"));
        assert!(generic_policy.contains("sched_yield"));
        assert!(!rhel_policy.contains("memfd_create"));
        assert!(!rhel_policy.contains("sched_yield"));
        assert!(!rhel_policy.contains("open,"));
        assert!(rhel_policy.contains("openat"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_process_lifecycle_and_rust_groups_by_linux_family() {
        let generic_policy = seccomp_policy_string_with_mode(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let debian_policy = seccomp_policy_string_with_mode(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
        );
        let rhel_policy = seccomp_policy_string_with_mode(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::RhelLike,
        );
        let compiler_debian_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::X86_64,
        });

        assert!(generic_policy.contains("wait4"));
        assert!(!generic_policy.contains("waitid"));
        assert!(!debian_policy.contains("wait4"));
        assert!(!debian_policy.contains("waitid"));
        assert!(rhel_policy.contains("waitid"));
        assert!(compiler_debian_policy.contains("waitid"));
        assert!(compiler_debian_policy.contains("wait4"));
        assert!(generic_policy.contains("sched_getaffinity"));
        assert!(!rhel_policy.contains("sched_getaffinity"));
        assert!(rhel_policy.contains("poll"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_python_and_cpp_runtime_extras_by_linux_family() {
        let cpp_generic_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let cpp_rhel_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::RhelLike,
        );
        let python_generic_policy = seccomp_policy_string_with_mode(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let python_debian_policy = seccomp_policy_string_with_mode(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
        );
        let python_rhel_policy = seccomp_policy_string_with_mode(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::RhelLike,
        );

        assert!(cpp_generic_policy.contains("ioctl"));
        assert!(!cpp_rhel_policy.contains("ioctl"));

        assert!(python_generic_policy.contains("sysinfo"));
        assert!(python_debian_policy.contains("readlink"));
        assert!(!python_rhel_policy.contains("sysinfo"));
        assert!(python_rhel_policy.contains("pipe2"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_runtime_core_and_compiler_extras_by_linux_family() {
        let generic_cpp_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::Generic,
        );
        let rhel_cpp_policy = seccomp_policy_string_with_mode(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::RhelLike,
        );
        let generic_compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::Generic,
            arch: RuntimeSyscallArch::X86_64,
        });
        let rhel_compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::RhelLike,
            arch: RuntimeSyscallArch::X86_64,
        });

        assert!(generic_cpp_policy.contains("readv"));
        assert!(generic_cpp_policy.contains("writev"));
        assert!(!rhel_cpp_policy.contains("readv"));
        assert!(!rhel_cpp_policy.contains("writev"));

        assert!(generic_compiler_policy.contains("sysinfo"));
        assert!(generic_compiler_policy.contains("pipe2"));
        assert!(!rhel_compiler_policy.contains("sysinfo"));
        assert!(rhel_compiler_policy.contains("pipe2"));
    }

    #[test]
    fn runtime_syscall_flavor_expands_compiler_exec_by_linux_family() {
        let generic_compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::Generic,
            arch: RuntimeSyscallArch::X86_64,
        });
        let debian_compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::X86_64,
        });
        let arch_compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::Arch,
            arch: RuntimeSyscallArch::X86_64,
        });
        let rhel_compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::RhelLike,
            arch: RuntimeSyscallArch::X86_64,
        });

        assert!(generic_compiler_policy.contains("execve"));
        assert!(!generic_compiler_policy.contains("execveat"));
        assert!(debian_compiler_policy.contains("execveat"));
        assert!(arch_compiler_policy.contains("execveat"));
        assert!(rhel_compiler_policy.contains("execve"));
        assert!(!rhel_compiler_policy.contains("execveat"));
    }

    #[test]
    fn runtime_syscall_arch_expands_rust_and_wasm_profiles_for_aarch64() {
        let rust_x64_policy = seccomp_policy_string_with_mode_and_arch(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::X86_64,
        );
        let rust_arm_policy = seccomp_policy_string_with_mode_and_arch(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_x64_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::X86_64,
        );
        let wasm_arm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );

        assert!(!rust_x64_policy.contains("ppoll"));
        assert!(rust_arm_policy.contains("ppoll"));
        assert!(!wasm_x64_policy.contains("epoll_pwait"));
        assert!(wasm_arm_policy.contains("epoll_pwait"));
        assert!(wasm_arm_policy.contains("membarrier"));
        assert!(wasm_arm_policy.contains("mremap"));
        assert!(wasm_arm_policy.contains("renameat"));
    }

    #[test]
    fn runtime_syscall_debian_runtime_profiles_drop_clone3_while_preserving_clone() {
        let python_policy = seccomp_policy_string_with_mode_and_arch(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::Aarch64,
        });

        assert!(python_policy.contains("clone"));
        assert!(!python_policy.contains("clone3"));
        assert!(wasm_policy.contains("clone"));
        assert!(!wasm_policy.contains("clone3"));
        assert!(compiler_policy.contains("clone3"));
    }

    #[test]
    fn runtime_syscall_debian_runtime_profiles_drop_waitid_while_preserving_compiler() {
        let cpp_policy = seccomp_policy_string_with_mode_and_arch(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let rust_policy = seccomp_policy_string_with_mode_and_arch(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::Aarch64,
        });

        assert!(!cpp_policy.contains("wait4"));
        assert!(!cpp_policy.contains("waitid"));
        assert!(!rust_policy.contains("wait4"));
        assert!(!rust_policy.contains("waitid"));
        assert!(!wasm_policy.contains("wait4"));
        assert!(!wasm_policy.contains("waitid"));
        assert!(compiler_policy.contains("wait4"));
        assert!(compiler_policy.contains("waitid"));
    }

    #[test]
    fn runtime_syscall_debian_aarch64_runtime_profiles_drop_pipe2_and_getdents64() {
        let python_arm_policy = seccomp_policy_string_with_mode_and_arch(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let python_x64_policy = seccomp_policy_string_with_mode_and_arch(
            "python_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::X86_64,
        );
        let wasm_arm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_x64_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::X86_64,
        );
        let compiler_arm_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::Aarch64,
        });

        assert!(!python_arm_policy.contains("getdents64"));
        assert!(!python_arm_policy.contains("pipe2"));
        assert!(python_x64_policy.contains("getdents64"));
        assert!(python_x64_policy.contains("pipe2"));
        assert!(!wasm_arm_policy.contains("getdents64"));
        assert!(!wasm_arm_policy.contains("pipe2"));
        assert!(wasm_x64_policy.contains("getdents64"));
        assert!(wasm_x64_policy.contains("pipe2"));
        assert!(compiler_arm_policy.contains("pipe2"));
    }

    #[test]
    fn runtime_syscall_debian_aarch64_wasm_profile_drops_remaining_sampled_extras() {
        let wasm_arm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_x64_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::X86_64,
        );
        let compiler_arm_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::Aarch64,
        });

        assert!(!wasm_arm_policy.contains("dup"));
        assert!(!wasm_arm_policy.contains("getpid"));
        assert!(!wasm_arm_policy.contains("readv"));
        assert!(!wasm_arm_policy.contains("sched_yield"));
        assert!(!wasm_arm_policy.contains("sysinfo"));
        assert!(!wasm_arm_policy.contains("writev"));
        assert!(wasm_x64_policy.contains("dup"));
        assert!(!wasm_x64_policy.contains("readv"));
        assert!(wasm_x64_policy.contains("sched_yield"));
        assert!(wasm_x64_policy.contains("sysinfo"));
        assert!(!wasm_x64_policy.contains("writev"));
        assert!(compiler_arm_policy.contains("dup"));
        assert!(compiler_arm_policy.contains("pread64"));
        assert!(compiler_arm_policy.contains("readv"));
        assert!(compiler_arm_policy.contains("sysinfo"));
        assert!(compiler_arm_policy.contains("writev"));
    }

    #[test]
    fn runtime_syscall_debian_runtime_profiles_drop_pread64_while_preserving_compiler() {
        let cpp_policy = seccomp_policy_string_with_mode_and_arch(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let rust_policy = seccomp_policy_string_with_mode_and_arch(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::Aarch64,
        });

        assert!(!cpp_policy.contains("pread64"));
        assert!(!rust_policy.contains("pread64"));
        assert!(!wasm_policy.contains("pread64"));
        assert!(compiler_policy.contains("pread64"));
    }

    #[test]
    fn runtime_syscall_debian_runtime_profiles_drop_clock_gettime_while_preserving_compiler() {
        let cpp_policy = seccomp_policy_string_with_mode_and_arch(
            "cpp_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let rust_policy = seccomp_policy_string_with_mode_and_arch(
            "rust_native_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_policy = seccomp_policy_string_with_mode_and_arch(
            "wasm_default",
            RuntimeSeccompMode::Log,
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let compiler_policy = compiler_seccomp_policy_string(RuntimeSyscallProfile {
            flavor: RuntimeSyscallFlavor::DebianUbuntu,
            arch: RuntimeSyscallArch::Aarch64,
        });

        assert!(!cpp_policy.contains("clock_gettime"));
        assert!(!rust_policy.contains("clock_gettime"));
        assert!(!wasm_policy.contains("clock_gettime"));
        assert!(compiler_policy.contains("clock_gettime"));
    }

    #[test]
    fn runtime_syscall_debian_runtime_profiles_drop_newfstat_while_preserving_compiler() {
        let cpp_syscalls = super::debug_seccomp_profile_syscalls_for_target(
            "cpp_native_default",
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let rust_syscalls = super::debug_seccomp_profile_syscalls_for_target(
            "rust_native_default",
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let wasm_syscalls = super::debug_seccomp_profile_syscalls_for_target(
            "wasm_default",
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );
        let compiler_syscalls = super::debug_seccomp_profile_syscalls_for_target(
            "compiler",
            RuntimeSyscallFlavor::DebianUbuntu,
            RuntimeSyscallArch::Aarch64,
        );

        assert!(!cpp_syscalls.contains(&"newfstat"));
        assert!(!rust_syscalls.contains(&"newfstat"));
        assert!(!wasm_syscalls.contains(&"newfstat"));
        assert!(compiler_syscalls.contains(&"newfstat"));
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

    fn wasm_runtime_task(language: &str) -> OjJudgeTask {
        OjJudgeTask {
            submission_id: SubmissionId::from("sub-wasm"),
            problem_id: ProblemId::from("p-wasm"),
            user_id: UserId::from("u-wasm"),
            language: language.to_owned(),
            judge_mode: RuntimeJudgeMode::Acm,
            sandbox_kind: RuntimeSandboxKind::Wasm,
            source_code: "fn main() {}".to_owned(),
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
        }
    }

    fn nsjail_wasm_runtime_task(language: &str) -> OjJudgeTask {
        let mut task = wasm_runtime_task(language);
        task.sandbox_kind = RuntimeSandboxKind::NsjailWasm;
        task
    }

    fn outcome_debug(outcome: &super::RuntimeExecutionOutcome) -> String {
        format!("{outcome:#?}")
    }

    fn assert_failed_with_single_case(outcome: &super::RuntimeExecutionOutcome) {
        assert!(
            matches!(outcome.final_status, RuntimeTaskLifecycleStatus::Failed),
            "unexpected outcome:\n{}",
            outcome_debug(outcome)
        );
        assert_eq!(
            outcome.cases.len(),
            1,
            "unexpected outcome:\n{}",
            outcome_debug(outcome)
        );
    }

    async fn execute_runtime_task(task: RuntimeTask) -> super::RuntimeExecutionOutcome {
        execute_runtime_task_with_seccomp_mode(task, RuntimeSeccompMode::Log).await
    }

    async fn execute_runtime_task_with_seccomp_mode(
        task: RuntimeTask,
        seccomp_mode: RuntimeSeccompMode,
    ) -> super::RuntimeExecutionOutcome {
        let nsjail = detect_nsjail_binary()
            .unwrap_or_else(|| panic!("skipping nsjail-backed test: missing nsjail binary"));
        let worker = RuntimeWorker::new(
            crate::build_default_runtime_catalog(),
            unique_test_workdir("runtime-safety"),
            nsjail,
            seccomp_mode,
            RuntimeSyscallFlavor::Generic,
            current_test_arch(),
        );
        let artifacts = worker.prepare(task).expect("artifacts should prepare");
        let outcome = worker
            .execute(&artifacts)
            .await
            .expect("execution should finish");
        let _ = fs::remove_dir_all(&artifacts.work_dir);
        outcome
    }

    fn runtime_task_with_source(
        language: &str,
        sandbox_kind: RuntimeSandboxKind,
        source_code: &str,
    ) -> RuntimeTask {
        runtime_task_with_source_and_limits(language, sandbox_kind, 1_000, 262_144, source_code)
    }

    fn runtime_task_with_source_and_limits(
        language: &str,
        sandbox_kind: RuntimeSandboxKind,
        time_limit_ms: u64,
        memory_limit_kb: u64,
        source_code: &str,
    ) -> RuntimeTask {
        let source_code = source_code.trim().to_owned();
        RuntimeTask {
            task_id: format!("task-safety-{language}-{sandbox_kind:?}"),
            task_type: RuntimeTaskType::OjJudge,
            source_domain: "oj".to_owned(),
            source_entity_id: format!("sub-safety-{language}"),
            queue: "oj_judge".to_owned(),
            lane: "fast".to_owned(),
            retry_policy: RuntimeRetryPolicy {
                max_attempts: 1,
                retry_delay_ms: 0,
            },
            payload: RuntimeTaskPayload::OjJudge(OjJudgeTask {
                submission_id: SubmissionId::from(format!("sub-safety-{language}")),
                problem_id: ProblemId::from(format!("p-safety-{language}")),
                user_id: UserId::from("u-safety"),
                language: language.to_owned(),
                judge_mode: RuntimeJudgeMode::Acm,
                sandbox_kind,
                source_code,
                limits: RuntimeLimits {
                    time_limit_ms,
                    memory_limit_kb,
                },
                testcases: vec![RuntimeTestcase {
                    case_no: 1,
                    input: "1 2\n".to_owned(),
                    expected_output: "3\n".to_owned(),
                    score: 100,
                }],
                judge_config: None,
            }),
        }
    }

    fn rust_wasm_toolchain_available() -> bool {
        command_exists("wasmtime") && rust_target_installed("wasm32-wasip1")
    }

    fn cpp_wasm_toolchain_available() -> bool {
        command_exists("wasmtime")
            && (command_exists("clang++-17") || command_exists("clang++"))
            && (command_exists("wasm-ld-17") || command_exists("wasm-ld"))
    }

    fn command_exists(command: &str) -> bool {
        StdCommand::new("/bin/sh")
            .args(["-lc", &format!("command -v {command} >/dev/null 2>&1")])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn command_path(command: &str) -> Option<String> {
        StdCommand::new("/bin/sh")
            .args(["-lc", &format!("command -v {command}")])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|output| output.trim().to_owned())
            .filter(|output| !output.is_empty())
    }

    fn detect_nsjail_binary() -> Option<String> {
        ["/usr/bin/nsjail", "/bin/nsjail"]
            .into_iter()
            .find(|candidate| Path::new(candidate).exists())
            .map(str::to_owned)
            .or_else(|| command_path("nsjail"))
    }

    fn current_test_arch() -> RuntimeSyscallArch {
        match std::env::consts::ARCH {
            "x86_64" => RuntimeSyscallArch::X86_64,
            "aarch64" => RuntimeSyscallArch::Aarch64,
            _ => RuntimeSyscallArch::Other,
        }
    }

    fn rust_target_installed(target: &str) -> bool {
        StdCommand::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .is_some_and(|output| output.lines().any(|line| line.trim() == target))
    }

    fn unique_test_workdir(label: &str) -> String {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        format!("/tmp/nexuscode-runtime-{label}-{millis}")
    }
}
