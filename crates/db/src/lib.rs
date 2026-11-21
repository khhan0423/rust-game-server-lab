//! Crate dedicated to DB access. SQL and storage access code lives only here (guidelines section 8).

use std::{
    num::{NonZeroU32, NonZeroU64},
    time::Duration,
};

use sqlx::migrate::{Migrate, MigrateError, Migrator};
use sqlx::postgres::PgPoolOptions;

pub mod login;

/// PostgreSQL connection pool.
pub type Pool = sqlx::PgPool;

/// Migrations applied with identical content to the Game DB of every shard.
pub static GAME_MIGRATOR: Migrator = sqlx::migrate!("../../migrations/game");

/// SQLSTATE code meaning "no such table".
const UNDEFINED_TABLE: &str = "42P01";

/// Why a DB operation failed.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Connection or query execution failed.
    #[error("database error")]
    Sqlx(#[from] sqlx::Error),
    /// The database returned rows that break a rule the schema and queries are meant to keep.
    #[error("database state is inconsistent: {0}")]
    Inconsistent(&'static str),
    /// A stored login identifier is not a UUID.
    #[error("invalid stored login identifier")]
    Identifier(#[from] uuid::Error),
    /// Applying or querying migrations failed.
    #[error("migration error")]
    Migrate(#[from] MigrateError),
    /// The migrations applied to the DB differ from the version this binary expects.
    #[error("schema is out of date: applied version {applied:?}, expected {expected}")]
    SchemaOutOfDate {
        /// Highest version applied to the DB. `None` if none are applied.
        applied: Option<i64>,
        /// Highest version embedded in the binary.
        expected: i64,
    },
}

/// Connection pool settings (design 04 section 3).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    max_connections: NonZeroU32,
    acquire_timeout_ms: NonZeroU64,
}

impl PoolConfig {
    async fn connect(&self, url: &str) -> Result<Pool, DbError> {
        Ok(PgPoolOptions::new()
            .max_connections(self.max_connections.get())
            .acquire_timeout(Duration::from_millis(self.acquire_timeout_ms.get()))
            .connect(url)
            .await?)
    }
}

/// Opens the configured pool and validates its schema before returning it.
pub async fn open(url: &str, config: &PoolConfig, migrator: &Migrator) -> Result<Pool, DbError> {
    let pool = config.connect(url).await?;
    check_schema(&pool, migrator).await?;
    Ok(pool)
}

/// Applies migrations through a dedicated pool and drains it before returning.
pub async fn migrate_url(
    url: &str,
    config: &PoolConfig,
    migrator: &Migrator,
) -> Result<(), DbError> {
    let pool = config.connect(url).await?;
    let result = migrate(&pool, migrator).await;
    pool.close().await;
    result
}

/// Applies pending migrations in order. Called by the deployment tool.
pub async fn migrate(pool: &Pool, migrator: &Migrator) -> Result<(), DbError> {
    migrator.run(pool).await?;
    Ok(())
}

/// Checks that the DB schema is at the same version as this binary's migrations. Does not modify the DB.
///
/// The server must pass this check at the startup barrier before accepting players (design 01 section 10.0).
pub async fn check_schema(pool: &Pool, migrator: &Migrator) -> Result<(), DbError> {
    let Some(expected) = migrator
        .iter()
        .filter(|migration| !migration.migration_type.is_down_migration())
        .map(|migration| migration.version)
        .max()
    else {
        return Ok(());
    };
    let mut connection = pool.acquire().await?;
    let applied = match connection
        .list_applied_migrations(&migrator.table_name)
        .await
    {
        Ok(migrations) => migrations.iter().map(|migration| migration.version).max(),
        Err(MigrateError::Execute(sqlx::Error::Database(error)))
            if error.code().as_deref() == Some(UNDEFINED_TABLE) =>
        {
            None
        }
        Err(error) => return Err(error.into()),
    };
    if applied == Some(expected) {
        Ok(())
    } else {
        Err(DbError::SchemaOutOfDate { applied, expected })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = false)]
    async fn check_schema_rejects_database_without_migrations(pool: Pool) {
        let result = check_schema(&pool, &GAME_MIGRATOR).await;

        assert!(matches!(
            result,
            Err(DbError::SchemaOutOfDate { applied: None, .. })
        ));
    }

    #[sqlx::test(migrations = false)]
    async fn check_schema_accepts_database_after_migrate(pool: Pool) {
        migrate(&pool, &GAME_MIGRATOR).await.unwrap();

        check_schema(&pool, &GAME_MIGRATOR).await.unwrap();
    }
}
