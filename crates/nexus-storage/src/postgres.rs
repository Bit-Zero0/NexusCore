use serde::{Deserialize, Serialize};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::path::Path;

use nexus_shared::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub max_connections: u32,
}

impl PostgresConfig {
    pub fn validate(&self) -> AppResult<()> {
        if self.host.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "postgres.host cannot be empty".to_owned(),
            ));
        }

        if self.database.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "postgres.database cannot be empty".to_owned(),
            ));
        }

        if self.username.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "postgres.username cannot be empty".to_owned(),
            ));
        }

        if self.max_connections == 0 {
            return Err(AppError::InvalidConfig(
                "postgres.max_connections must be greater than 0".to_owned(),
            ));
        }

        Ok(())
    }

    pub fn connect_options(&self) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(&self.host)
            .port(self.port)
            .database(&self.database)
            .username(&self.username)
            .password(&self.password)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PostgresDsn {
    pub value: String,
}

impl From<&PostgresConfig> for PostgresDsn {
    fn from(config: &PostgresConfig) -> Self {
        Self {
            value: format!(
                "postgres://{}:{}@{}:{}/{}",
                config.username, config.password, config.host, config.port, config.database
            ),
        }
    }
}

pub type PostgresPool = PgPool;

pub struct PostgresPoolFactory;

impl PostgresPoolFactory {
    pub async fn connect(config: &PostgresConfig) -> AppResult<PostgresPool> {
        config.validate()?;

        PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect_with(config.connect_options())
            .await
            .map_err(|err| AppError::Database(err.to_string()))
    }

    pub async fn ping(pool: &PostgresPool) -> AppResult<()> {
        sqlx::query("select 1")
            .execute(pool)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(())
    }

    pub async fn migrate(pool: &PostgresPool, migrations_path: &Path) -> AppResult<()> {
        let migrator = Migrator::new(migrations_path)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        migrator
            .run(pool)
            .await
            .map_err(|err| AppError::Database(err.to_string()))?;

        Ok(())
    }
}
