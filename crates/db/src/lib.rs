//! DB 접근 전담 크레이트. SQL과 저장소 접근 코드는 여기에만 둔다 (가이드 8장).

use std::time::Duration;

use sqlx::migrate::{Migrate, MigrateError, Migrator};
use sqlx::postgres::PgPoolOptions;

/// PostgreSQL 커넥션 풀.
pub type Pool = sqlx::PgPool;

/// 모든 샤드의 Game DB에 같은 내용으로 적용하는 마이그레이션.
pub static GAME_MIGRATOR: Migrator = sqlx::migrate!("../../migrations/game");

/// "그런 테이블이 없다"를 뜻하는 SQLSTATE 코드.
const UNDEFINED_TABLE: &str = "42P01";

/// DB 작업이 실패한 이유.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// 연결이나 쿼리 실행 실패.
    #[error("database error")]
    Sqlx(#[from] sqlx::Error),
    /// 마이그레이션 적용이나 조회 실패.
    #[error("migration error")]
    Migrate(#[from] MigrateError),
    /// DB에 적용된 마이그레이션이 이 바이너리가 기대하는 버전과 다르다.
    #[error("schema is out of date: applied version {applied:?}, expected {expected}")]
    SchemaOutOfDate {
        /// DB에 적용된 가장 높은 버전. 하나도 없으면 `None`.
        applied: Option<i64>,
        /// 바이너리에 들어 있는 가장 높은 버전.
        expected: i64,
    },
}

/// 커넥션 풀 설정 (설계 04 3장).
#[derive(Debug, Clone, Copy)]
pub struct PoolSettings {
    /// 동시에 여는 커넥션 수의 상한.
    pub max_connections: u32,
    /// 커넥션을 얻으려고 기다리는 최대 시간. 처음 접속에도 적용된다.
    pub acquire_timeout: Duration,
}

/// DB에 접속해 풀을 만든다. 접속에 실패하면 에러를 돌려준다.
pub async fn connect(url: &str, settings: PoolSettings) -> Result<Pool, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .acquire_timeout(settings.acquire_timeout)
        .connect(url)
        .await?;
    Ok(pool)
}

/// 아직 적용되지 않은 마이그레이션을 차례로 적용한다. 배포 도구가 부른다.
pub async fn migrate(pool: &Pool, migrator: &Migrator) -> Result<(), DbError> {
    migrator.run(pool).await?;
    Ok(())
}

/// DB 스키마가 이 바이너리의 마이그레이션과 같은 버전인지 확인한다. DB를 바꾸지 않는다.
///
/// 서버는 기동 배리어에서 이 확인을 통과해야 손님을 받는다(설계 01 10.0).
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
