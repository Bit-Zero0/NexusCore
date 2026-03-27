use std::{env, net::SocketAddr};

use nexus_shared::{AppError, AppResult};
use nexus_storage::PostgresConfig;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_env: String,
    pub process_role: AppProcessRole,
    pub oj_repository: OjRepositoryMode,
    pub postgres: PostgresConfig,
    pub redis: RedisConfig,
    pub server: ServerConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub cors_allowed_origins: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub work_root: String,
    pub nsjail_path: String,
    pub node_id: String,
    pub seccomp_mode: RuntimeSeccompMode,
    pub syscall_flavor: RuntimeSyscallFlavor,
    pub syscall_arch: RuntimeSyscallArch,
    pub queue_backend: RuntimeQueueBackend,
    pub rabbitmq: RabbitMqRuntimeConfig,
    pub worker_groups: Vec<RuntimeWorkerGroupConfig>,
}

#[derive(Debug, Clone)]
pub struct RedisConfig {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct RabbitMqRuntimeConfig {
    pub url: String,
    pub exchange: String,
    pub queue_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRouteBindingConfig {
    pub queue: String,
    pub lane: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWorkerGroupConfig {
    pub name: String,
    pub bindings: Vec<RuntimeRouteBindingConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppProcessRole {
    Embedded,
    Gateway,
    RuntimeWorker,
}

impl AppProcessRole {
    fn from_env(value: &str) -> AppResult<Self> {
        match value {
            "embedded" => Ok(Self::Embedded),
            "gateway" => Ok(Self::Gateway),
            "runtime-worker" => Ok(Self::RuntimeWorker),
            other => Err(AppError::InvalidConfig(format!(
                "NEXUS_PROCESS_ROLE must be 'embedded', 'gateway', or 'runtime-worker', got: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeQueueBackend {
    Memory,
    RabbitMq,
}

impl RuntimeQueueBackend {
    fn from_env(value: &str) -> AppResult<Self> {
        match value {
            "memory" => Ok(Self::Memory),
            "rabbitmq" => Ok(Self::RabbitMq),
            other => Err(AppError::InvalidConfig(format!(
                "NEXUS_RUNTIME_QUEUE_BACKEND must be 'memory' or 'rabbitmq', got: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSeccompMode {
    Log,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSyscallFlavor {
    Auto,
    Generic,
    DebianUbuntu,
    Arch,
    RhelLike,
}

impl RuntimeSyscallFlavor {
    fn from_env(value: &str) -> AppResult<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "generic" => Ok(Self::Generic),
            "debian_ubuntu" => Ok(Self::DebianUbuntu),
            "arch" => Ok(Self::Arch),
            "rhel_like" => Ok(Self::RhelLike),
            other => Err(AppError::InvalidConfig(format!(
                "NEXUS_RUNTIME_SYSCALL_FLAVOR must be 'auto', 'generic', 'debian_ubuntu', 'arch', or 'rhel_like', got: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSyscallArch {
    Auto,
    X86_64,
    Aarch64,
    Other,
}

impl RuntimeSyscallArch {
    fn from_env(value: &str) -> AppResult<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "x86_64" => Ok(Self::X86_64),
            "aarch64" => Ok(Self::Aarch64),
            "other" => Ok(Self::Other),
            other => Err(AppError::InvalidConfig(format!(
                "NEXUS_RUNTIME_SYSCALL_ARCH must be 'auto', 'x86_64', 'aarch64', or 'other', got: {other}"
            ))),
        }
    }
}

impl RuntimeSeccompMode {
    fn from_env(value: &str) -> AppResult<Self> {
        match value {
            "log" => Ok(Self::Log),
            "kill" => Ok(Self::Kill),
            other => Err(AppError::InvalidConfig(format!(
                "NEXUS_RUNTIME_SECCOMP_MODE must be 'log' or 'kill', got: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OjRepositoryMode {
    Memory,
    Postgres,
}

impl OjRepositoryMode {
    fn from_env(value: &str) -> AppResult<Self> {
        match value {
            "memory" => Ok(Self::Memory),
            "postgres" => Ok(Self::Postgres),
            other => Err(AppError::InvalidConfig(format!(
                "NEXUS_OJ_REPOSITORY must be 'memory' or 'postgres', got: {other}"
            ))),
        }
    }
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        let app_env = env::var("NEXUS_ENV").unwrap_or_else(|_| "dev".to_owned());
        let oj_repository = OjRepositoryMode::from_env(
            &env::var("NEXUS_OJ_REPOSITORY").unwrap_or_else(|_| "memory".to_owned()),
        )?;
        let bind_addr = env::var("NEXUS_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
            .parse()
            .map_err(|err| AppError::InvalidConfig(format!("NEXUS_BIND_ADDR: {err}")))?;
        let cors_allowed_origins = cors_allowed_origins_from_env(
            env::var("NEXUS_CORS_ALLOWED_ORIGINS").ok().as_deref(),
            &app_env,
        );
        let postgres = PostgresConfig {
            host: env::var("NEXUS_PG_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned()),
            port: env::var("NEXUS_PG_PORT")
                .unwrap_or_else(|_| "5432".to_owned())
                .parse()
                .map_err(|err| AppError::InvalidConfig(format!("NEXUS_PG_PORT: {err}")))?,
            database: env::var("NEXUS_PG_DATABASE").unwrap_or_else(|_| "nexus_code".to_owned()),
            username: env::var("NEXUS_PG_USERNAME").unwrap_or_else(|_| "postgres".to_owned()),
            password: env::var("NEXUS_PG_PASSWORD").unwrap_or_else(|_| "postgres".to_owned()),
            max_connections: env::var("NEXUS_PG_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "20".to_owned())
                .parse()
                .map_err(|err| {
                    AppError::InvalidConfig(format!("NEXUS_PG_MAX_CONNECTIONS: {err}"))
                })?,
        };
        postgres.validate()?;

        Ok(Self {
            app_env,
            process_role: AppProcessRole::from_env(
                &env::var("NEXUS_PROCESS_ROLE").unwrap_or_else(|_| "embedded".to_owned()),
            )?,
            oj_repository,
            postgres,
            redis: RedisConfig {
                url: env::var("NEXUS_REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_owned()),
            },
            server: ServerConfig {
                bind_addr,
                cors_allowed_origins,
            },
            runtime: RuntimeConfig {
                work_root: env::var("NEXUS_RUNTIME_WORK_ROOT")
                    .unwrap_or_else(|_| "/tmp/nexuscode-runtime".to_owned()),
                nsjail_path: env::var("NEXUS_RUNTIME_NSJAIL_PATH")
                    .unwrap_or_else(|_| "/usr/bin/nsjail".to_owned()),
                node_id: env::var("NEXUS_RUNTIME_NODE_ID")
                    .unwrap_or_else(|_| default_runtime_node_id()),
                seccomp_mode: RuntimeSeccompMode::from_env(
                    &env::var("NEXUS_RUNTIME_SECCOMP_MODE").unwrap_or_else(|_| "log".to_owned()),
                )?,
                syscall_flavor: RuntimeSyscallFlavor::from_env(
                    &env::var("NEXUS_RUNTIME_SYSCALL_FLAVOR").unwrap_or_else(|_| "auto".to_owned()),
                )?,
                syscall_arch: RuntimeSyscallArch::from_env(
                    &env::var("NEXUS_RUNTIME_SYSCALL_ARCH").unwrap_or_else(|_| "auto".to_owned()),
                )?,
                queue_backend: RuntimeQueueBackend::from_env(
                    &env::var("NEXUS_RUNTIME_QUEUE_BACKEND")
                        .unwrap_or_else(|_| "memory".to_owned()),
                )?,
                rabbitmq: RabbitMqRuntimeConfig {
                    url: env::var("NEXUS_RUNTIME_RABBITMQ_URL")
                        .unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/%2f".to_owned()),
                    exchange: env::var("NEXUS_RUNTIME_RABBITMQ_EXCHANGE")
                        .unwrap_or_else(|_| "nexus.runtime".to_owned()),
                    queue_prefix: env::var("NEXUS_RUNTIME_RABBITMQ_QUEUE_PREFIX")
                        .unwrap_or_else(|_| "nexus.runtime".to_owned()),
                },
                worker_groups: runtime_worker_groups_from_env(
                    env::var("NEXUS_RUNTIME_WORKER_GROUPS").ok().as_deref(),
                )?,
            },
        })
    }
}

fn cors_allowed_origins_from_env(value: Option<&str>, app_env: &str) -> Vec<String> {
    let parsed = value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !parsed.is_empty() {
        return parsed;
    }

    if app_env == "dev" {
        return vec![
            "http://localhost:5173".to_owned(),
            "http://127.0.0.1:5173".to_owned(),
            "http://localhost:4173".to_owned(),
            "http://127.0.0.1:4173".to_owned(),
        ];
    }

    Vec::new()
}

fn default_runtime_node_id() -> String {
    let hostname = env::var("HOSTNAME").unwrap_or_else(|_| "runtime-node".to_owned());
    format!("{hostname}-{}", std::process::id())
}

fn runtime_worker_groups_from_env(value: Option<&str>) -> AppResult<Vec<RuntimeWorkerGroupConfig>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default_runtime_worker_groups_config());
    };

    let mut worker_groups = Vec::new();
    for segment in value
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (name, bindings) = segment.split_once('=').ok_or_else(|| {
            AppError::InvalidConfig(format!(
                "invalid NEXUS_RUNTIME_WORKER_GROUPS segment: {segment}"
            ))
        })?;
        let bindings = bindings
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(parse_runtime_route_binding)
            .collect::<AppResult<Vec<_>>>()?;
        if bindings.is_empty() {
            return Err(AppError::InvalidConfig(format!(
                "worker group {name} must contain at least one binding"
            )));
        }
        worker_groups.push(RuntimeWorkerGroupConfig {
            name: name.trim().to_owned(),
            bindings,
        });
    }

    if worker_groups.is_empty() {
        return Err(AppError::InvalidConfig(
            "NEXUS_RUNTIME_WORKER_GROUPS must not be empty".to_owned(),
        ));
    }

    Ok(worker_groups)
}

fn parse_runtime_route_binding(value: &str) -> AppResult<RuntimeRouteBindingConfig> {
    let (queue, lane) = value.split_once(':').ok_or_else(|| {
        AppError::InvalidConfig(format!(
            "invalid runtime worker binding, expected queue:lane, got: {value}"
        ))
    })?;
    if queue.trim().is_empty() || lane.trim().is_empty() {
        return Err(AppError::InvalidConfig(format!(
            "invalid runtime worker binding, queue/lane cannot be empty: {value}"
        )));
    }
    Ok(RuntimeRouteBindingConfig {
        queue: queue.trim().to_owned(),
        lane: lane.trim().to_owned(),
    })
}

fn default_runtime_worker_groups_config() -> Vec<RuntimeWorkerGroupConfig> {
    vec![
        runtime_worker_group_config("oj-fast", &[("oj_judge", "fast")]),
        runtime_worker_group_config("oj-normal", &[("oj_judge", "normal")]),
        runtime_worker_group_config("oj-heavy", &[("oj_judge", "heavy")]),
        runtime_worker_group_config("oj-special", &[("oj_judge", "special")]),
    ]
}

fn runtime_worker_group_config(name: &str, bindings: &[(&str, &str)]) -> RuntimeWorkerGroupConfig {
    RuntimeWorkerGroupConfig {
        name: name.to_owned(),
        bindings: bindings
            .iter()
            .map(|(queue, lane)| RuntimeRouteBindingConfig {
                queue: (*queue).to_owned(),
                lane: (*lane).to_owned(),
            })
            .collect(),
    }
}
