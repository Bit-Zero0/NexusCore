use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command as StdCommand,
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use nexus_shared::{AppError, AppResult};

use crate::protocol::{OjJudgeTask, RuntimeSandboxKind};

pub trait LanguageRuntimeSpec: Send + Sync {
    fn key(&self) -> &'static str;
    fn build_plan(&self, task: &OjJudgeTask) -> AppResult<RuntimeExecutionPlan>;
}

#[derive(Clone, Default)]
pub struct RuntimeLanguageCatalog {
    specs: HashMap<String, Arc<dyn LanguageRuntimeSpec>>,
}

impl RuntimeLanguageCatalog {
    pub fn new(specs: Vec<Arc<dyn LanguageRuntimeSpec>>) -> Self {
        let specs = specs
            .into_iter()
            .map(|spec| (spec.key().to_owned(), spec))
            .collect();
        Self { specs }
    }

    pub fn resolve(&self, language: &str) -> Option<Arc<dyn LanguageRuntimeSpec>> {
        self.specs.get(language).cloned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExecutionBackend {
    NsjailNative,
    WasmWasi,
    NsjailWasm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeExecutionPlan {
    pub language: String,
    pub execution_backend: RuntimeExecutionBackend,
    pub compile_required: bool,
    pub source_filename: String,
    pub executable_filename: Option<String>,
    pub compile_command: Vec<String>,
    pub run_command: Vec<String>,
    pub sandbox_profile: String,
    pub seccomp_policy: String,
    pub readonly_mounts: Vec<String>,
}

#[derive(Debug, Clone)]
struct RuntimeCommandRecipe {
    language: String,
    compile_required: bool,
    source_filename: String,
    executable_filename: Option<String>,
    compile_command: Vec<String>,
    run_command: Vec<String>,
    seccomp_policy: String,
    readonly_mounts: Vec<String>,
}

trait LanguageToolchain: Send + Sync {
    fn key(&self) -> &'static str;
    fn build_recipe(
        &self,
        task: &OjJudgeTask,
        sandbox_kind: RuntimeSandboxKind,
    ) -> AppResult<RuntimeCommandRecipe>;
}

trait SandboxBackend: Send + Sync {
    fn kind(&self) -> RuntimeSandboxKind;
    fn profile_name(&self) -> &'static str;
    fn execution_backend(&self) -> RuntimeExecutionBackend;
    fn finalize_plan(&self, recipe: RuntimeCommandRecipe) -> RuntimeExecutionPlan {
        RuntimeExecutionPlan {
            language: recipe.language,
            execution_backend: self.execution_backend(),
            compile_required: recipe.compile_required,
            source_filename: recipe.source_filename,
            executable_filename: recipe.executable_filename,
            compile_command: recipe.compile_command,
            run_command: recipe.run_command,
            sandbox_profile: self.profile_name().to_owned(),
            seccomp_policy: recipe.seccomp_policy,
            readonly_mounts: recipe.readonly_mounts,
        }
    }
}

struct RuntimeExecutionPlanner<T: LanguageToolchain> {
    toolchain: T,
    sandboxes: HashMap<RuntimeSandboxKind, Arc<dyn SandboxBackend>>,
}

impl<T: LanguageToolchain> RuntimeExecutionPlanner<T> {
    fn new(toolchain: T) -> Self {
        let sandboxes = vec![
            Arc::new(NsjailSandboxBackend) as Arc<dyn SandboxBackend>,
            Arc::new(WasmSandboxBackend) as Arc<dyn SandboxBackend>,
            Arc::new(NsjailWasmSandboxBackend) as Arc<dyn SandboxBackend>,
        ]
        .into_iter()
        .map(|backend| (backend.kind(), backend))
        .collect();
        Self {
            toolchain,
            sandboxes,
        }
    }
}

impl<T: LanguageToolchain> LanguageRuntimeSpec for RuntimeExecutionPlanner<T> {
    fn key(&self) -> &'static str {
        self.toolchain.key()
    }

    fn build_plan(&self, task: &OjJudgeTask) -> AppResult<RuntimeExecutionPlan> {
        let backend = self
            .sandboxes
            .get(&task.sandbox_kind)
            .ok_or_else(|| AppError::BadRequest("unsupported sandbox backend".to_owned()))?;
        let recipe = self.toolchain.build_recipe(task, task.sandbox_kind)?;
        Ok(backend.finalize_plan(recipe))
    }
}

struct NsjailSandboxBackend;

impl SandboxBackend for NsjailSandboxBackend {
    fn kind(&self) -> RuntimeSandboxKind {
        RuntimeSandboxKind::Nsjail
    }

    fn profile_name(&self) -> &'static str {
        "nsjail"
    }

    fn execution_backend(&self) -> RuntimeExecutionBackend {
        RuntimeExecutionBackend::NsjailNative
    }
}

struct WasmSandboxBackend;

impl SandboxBackend for WasmSandboxBackend {
    fn kind(&self) -> RuntimeSandboxKind {
        RuntimeSandboxKind::Wasm
    }

    fn profile_name(&self) -> &'static str {
        "wasm"
    }

    fn execution_backend(&self) -> RuntimeExecutionBackend {
        RuntimeExecutionBackend::WasmWasi
    }
}

struct NsjailWasmSandboxBackend;

impl SandboxBackend for NsjailWasmSandboxBackend {
    fn kind(&self) -> RuntimeSandboxKind {
        RuntimeSandboxKind::NsjailWasm
    }

    fn profile_name(&self) -> &'static str {
        "nsjail_wasm"
    }

    fn execution_backend(&self) -> RuntimeExecutionBackend {
        RuntimeExecutionBackend::NsjailWasm
    }
}

struct CppToolchain;

impl LanguageToolchain for CppToolchain {
    fn key(&self) -> &'static str {
        "cpp"
    }

    fn build_recipe(
        &self,
        task: &OjJudgeTask,
        sandbox_kind: RuntimeSandboxKind,
    ) -> AppResult<RuntimeCommandRecipe> {
        let recipe = match sandbox_kind {
            RuntimeSandboxKind::Nsjail => RuntimeCommandRecipe {
                language: "cpp".to_owned(),
                compile_required: true,
                source_filename: "main.cpp".to_owned(),
                executable_filename: Some("main".to_owned()),
                compile_command: vec![
                    resolve_native_cpp_compiler(),
                    "-std=c++20".to_owned(),
                    "-O2".to_owned(),
                    "-pipe".to_owned(),
                    "-o".to_owned(),
                    "main".to_owned(),
                    "main.cpp".to_owned(),
                ],
                run_command: vec!["./main".to_owned()],
                seccomp_policy: "cpp_native_default".to_owned(),
                readonly_mounts: Vec::new(),
            },
            RuntimeSandboxKind::Wasm | RuntimeSandboxKind::NsjailWasm => RuntimeCommandRecipe {
                language: "cpp".to_owned(),
                compile_required: true,
                source_filename: "main.cpp".to_owned(),
                executable_filename: Some("main.wasm".to_owned()),
                compile_command: vec![
                    resolve_wasi_cpp_compiler(),
                    "--target=wasm32-wasi".to_owned(),
                    "-std=c++20".to_owned(),
                    "-O2".to_owned(),
                    "-fno-exceptions".to_owned(),
                    "-D_LIBCPP_NO_EXCEPTIONS".to_owned(),
                    "-o".to_owned(),
                    "main.wasm".to_owned(),
                    "main.cpp".to_owned(),
                ],
                run_command: build_wasmtime_run_command(task),
                seccomp_policy: "wasm_default".to_owned(),
                readonly_mounts: wasi_runtime_mounts(),
            },
        };
        Ok(recipe)
    }
}

struct PythonToolchain;

impl LanguageToolchain for PythonToolchain {
    fn key(&self) -> &'static str {
        "python"
    }

    fn build_recipe(
        &self,
        _task: &OjJudgeTask,
        sandbox_kind: RuntimeSandboxKind,
    ) -> AppResult<RuntimeCommandRecipe> {
        if matches!(
            sandbox_kind,
            RuntimeSandboxKind::Wasm | RuntimeSandboxKind::NsjailWasm
        ) {
            return Err(AppError::BadRequest(
                "python does not support wasm sandbox".to_owned(),
            ));
        }

        Ok(RuntimeCommandRecipe {
            language: "python".to_owned(),
            compile_required: false,
            source_filename: "main.py".to_owned(),
            executable_filename: None,
            compile_command: Vec::new(),
            run_command: vec![resolve_python_runtime(), "main.py".to_owned()],
            seccomp_policy: "python_default".to_owned(),
            readonly_mounts: Vec::new(),
        })
    }
}

struct RustToolchain;

impl LanguageToolchain for RustToolchain {
    fn key(&self) -> &'static str {
        "rust"
    }

    fn build_recipe(
        &self,
        task: &OjJudgeTask,
        sandbox_kind: RuntimeSandboxKind,
    ) -> AppResult<RuntimeCommandRecipe> {
        let recipe = match sandbox_kind {
            RuntimeSandboxKind::Nsjail => {
                let rustc = resolve_rustc_binary();
                let readonly_mounts = rustc_mounts(&rustc);
                RuntimeCommandRecipe {
                    language: "rust".to_owned(),
                    compile_required: true,
                    source_filename: "main.rs".to_owned(),
                    executable_filename: Some("main".to_owned()),
                    compile_command: vec![
                        rustc,
                        "-O".to_owned(),
                        "-C".to_owned(),
                        "linker=/usr/bin/cc".to_owned(),
                        "-C".to_owned(),
                        "link-arg=-fuse-ld=bfd".to_owned(),
                        "-o".to_owned(),
                        "main".to_owned(),
                        "main.rs".to_owned(),
                    ],
                    run_command: vec!["./main".to_owned()],
                    seccomp_policy: "rust_native_default".to_owned(),
                    readonly_mounts,
                }
            }
            RuntimeSandboxKind::Wasm | RuntimeSandboxKind::NsjailWasm => {
                let rustc = resolve_rustc_binary();
                let mut readonly_mounts = rustc_mounts(&rustc);
                readonly_mounts.extend(wasi_runtime_mounts());
                readonly_mounts.sort();
                readonly_mounts.dedup();
                RuntimeCommandRecipe {
                    language: "rust".to_owned(),
                    compile_required: true,
                    source_filename: "main.rs".to_owned(),
                    executable_filename: Some("main.wasm".to_owned()),
                    compile_command: vec![
                        rustc,
                        "--target".to_owned(),
                        resolve_wasi_rust_target(),
                        "-O".to_owned(),
                        "-o".to_owned(),
                        "main.wasm".to_owned(),
                        "main.rs".to_owned(),
                    ],
                    run_command: build_wasmtime_run_command(task),
                    seccomp_policy: "wasm_default".to_owned(),
                    readonly_mounts,
                }
            }
        };
        Ok(recipe)
    }
}

pub(crate) struct CppRuntimeSpec(RuntimeExecutionPlanner<CppToolchain>);

impl Default for CppRuntimeSpec {
    fn default() -> Self {
        Self(RuntimeExecutionPlanner::new(CppToolchain))
    }
}

impl LanguageRuntimeSpec for CppRuntimeSpec {
    fn key(&self) -> &'static str {
        self.0.key()
    }

    fn build_plan(&self, task: &OjJudgeTask) -> AppResult<RuntimeExecutionPlan> {
        self.0.build_plan(task)
    }
}

pub(crate) struct PythonRuntimeSpec(RuntimeExecutionPlanner<PythonToolchain>);

impl Default for PythonRuntimeSpec {
    fn default() -> Self {
        Self(RuntimeExecutionPlanner::new(PythonToolchain))
    }
}

impl LanguageRuntimeSpec for PythonRuntimeSpec {
    fn key(&self) -> &'static str {
        self.0.key()
    }

    fn build_plan(&self, task: &OjJudgeTask) -> AppResult<RuntimeExecutionPlan> {
        self.0.build_plan(task)
    }
}

pub(crate) struct RustRuntimeSpec(RuntimeExecutionPlanner<RustToolchain>);

impl Default for RustRuntimeSpec {
    fn default() -> Self {
        Self(RuntimeExecutionPlanner::new(RustToolchain))
    }
}

impl LanguageRuntimeSpec for RustRuntimeSpec {
    fn key(&self) -> &'static str {
        self.0.key()
    }

    fn build_plan(&self, task: &OjJudgeTask) -> AppResult<RuntimeExecutionPlan> {
        self.0.build_plan(task)
    }
}

pub fn build_default_runtime_catalog() -> RuntimeLanguageCatalog {
    RuntimeLanguageCatalog::new(vec![
        Arc::new(CppRuntimeSpec::default()),
        Arc::new(PythonRuntimeSpec::default()),
        Arc::new(RustRuntimeSpec::default()),
    ])
}

pub(crate) fn resolve_native_cpp_compiler() -> String {
    if let Ok(value) = env::var("NEXUS_RUNTIME_CPP_COMPILER") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    resolve_existing_path(["/usr/bin/g++", "/bin/g++"]).unwrap_or_else(|| "g++".to_owned())
}

fn resolve_wasi_cpp_compiler() -> String {
    if let Ok(value) = env::var("NEXUS_RUNTIME_WASI_CXX") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    resolve_existing_path([
        "/usr/bin/clang++-17",
        "/bin/clang++-17",
        "/usr/bin/clang++",
        "/bin/clang++",
    ])
    .unwrap_or_else(|| "clang++".to_owned())
}

pub(crate) fn resolve_python_runtime() -> String {
    resolve_existing_path(["/usr/bin/python3", "/bin/python3"])
        .unwrap_or_else(|| "python3".to_owned())
}

pub(crate) fn resolve_rustc_binary() -> String {
    if let Ok(value) = env::var("NEXUS_RUNTIME_RUSTC") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        let rustup_toolchains = home.join(".rustup/toolchains");
        if let Ok(entries) = fs::read_dir(&rustup_toolchains) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin/rustc");
                if candidate.exists() {
                    return candidate.display().to_string();
                }
            }
        }

        let rustup = home.join(".cargo/bin/rustup");
        if rustup.exists() {
            if let Ok(output) = StdCommand::new(&rustup).args(["which", "rustc"]).output() {
                if output.status.success() {
                    let resolved = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    if !resolved.is_empty() {
                        return resolved;
                    }
                }
            }
        }
    }

    "rustc".to_owned()
}

fn resolve_wasi_rust_target() -> String {
    if let Ok(value) = env::var("NEXUS_RUNTIME_WASI_RUST_TARGET") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    "wasm32-wasip1".to_owned()
}

fn resolve_wasmtime_runtime() -> String {
    if let Ok(value) = env::var("NEXUS_RUNTIME_WASMTIME") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    if let Some(home) = env::var_os("HOME") {
        let candidate = PathBuf::from(home).join(".wasmtime/bin/wasmtime");
        if candidate.exists() {
            return candidate.display().to_string();
        }
    }

    resolve_existing_path([
        "/usr/local/bin/wasmtime",
        "/usr/bin/wasmtime",
        "/bin/wasmtime",
    ])
    .unwrap_or_else(|| "wasmtime".to_owned())
}

fn wasi_runtime_mounts() -> Vec<String> {
    let wasmtime = resolve_wasmtime_runtime();
    let path = Path::new(&wasmtime);
    if !path.is_absolute() {
        return Vec::new();
    }

    let mut mounts = Vec::new();
    if let Some(parent) = path.parent() {
        mounts.push(parent.display().to_string());
    }
    mounts
}

fn build_wasmtime_run_command(task: &OjJudgeTask) -> Vec<String> {
    vec![
        resolve_wasmtime_runtime(),
        "run".to_owned(),
        "-W".to_owned(),
        format!("max-memory-size={}", task.limits.memory_limit_kb * 1024),
        "-W".to_owned(),
        format!("timeout={}ms", task.limits.time_limit_ms.max(1)),
        "main.wasm".to_owned(),
    ]
}

pub(crate) fn rustc_mounts(rustc: &str) -> Vec<String> {
    let path = Path::new(rustc);
    if !path.is_absolute() {
        return Vec::new();
    }

    let mut mounts = Vec::new();
    if let Some(toolchain_root) = path.parent().and_then(Path::parent) {
        mounts.push(toolchain_root.display().to_string());
    }

    if let Some(home) = env::var_os("HOME") {
        let cargo_bin = PathBuf::from(home).join(".cargo/bin");
        if cargo_bin.exists() {
            mounts.push(cargo_bin.display().to_string());
        }
    }

    mounts.sort();
    mounts.dedup();
    mounts
}

fn resolve_existing_path<const N: usize>(candidates: [&str; N]) -> Option<String> {
    candidates
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .map(str::to_owned)
}
