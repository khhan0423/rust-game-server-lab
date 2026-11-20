//! Crate dedicated to DB access. SQL and storage access code lives only here (guidelines section 8).

use std::time::Duration;

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
#[derive(Debug, Clone, Copy)]
pub struct PoolSettings {
    /// Upper bound on the number of connections open at the same time.
    pub max_connections: u32,
    /// Maximum time to wait to acquire a connection. Also applies to the first connection.
    pub acquire_timeout: Duration,
}

/// Connects to the DB and creates a pool. Returns an error if the connection fails.
pub async fn connect(url: &str, settings: PoolSettings) -> Result<Pool, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .acquire_timeout(settings.acquire_timeout)
        .connect(url)
        .await?;
    Ok(pool)
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
