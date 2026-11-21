//! Startup barrier of the Game server: it signals ready only after preparation finishes, and does not start if preparation fails.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use sqlx::{ConnectOptions, PgPool};
use std::net::SocketAddr;
use testkit::{ServerProcess, TestDir};
use tokio::process::Command;

const DB_URL_ENV: &str = "GAME_DB_URL";

async fn server(name: &str, command: &str) -> Command {
    let dir = TestDir::new(env!("CARGO_TARGET_TMPDIR"), name)
        .await
        .unwrap();
    let contents = format!(
        "[process]\nname = 'game'\nenvironment = 'development'\nadmin_addr = \"127.0.0.1:0\"\nlog_root = '{}'\nlog_level = \"info\"\n\n\
         [db]\nmax_connections = 2\nacquire_timeout_ms = 3000\n",
        dir.path().join("logs").display()
    );
    let config = dir.write("server.toml", &contents).await.unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_server"));
    process
        .arg("--config")
        .arg(config)
        .arg(command)
        .env_remove(DB_URL_ENV);
    process
}

fn database_url(pool: &PgPool) -> String {
    pool.connect_options().to_url_lossy().to_string()
}

async fn get(addr: SocketAddr, path: &str) -> reqwest::Response {
    reqwest::get(format!("http://{addr}{path}")).await.unwrap()
}

#[sqlx::test(migrator = "db::GAME_MIGRATOR")]
async fn server_announces_ready_when_schema_is_current(pool: PgPool) {
    let mut command = server("ready", "run").await;
    command.env(DB_URL_ENV, database_url(&pool));
    let mut child = ServerProcess::spawn(&mut command).await.unwrap();
    let admin = child.addr("admin").unwrap();

    assert_eq!(get(admin, "/health/live").await.status(), 200);
    assert_eq!(get(admin, "/health/ready").await.status(), 200);
    let metrics = get(admin, "/metrics").await.text().await.unwrap();
    assert!(
        metrics.lines().any(|line| line == "server_ready 1"),
        "unexpected metrics: {metrics}"
    );
    child.stop().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn server_refuses_to_start_when_schema_is_out_of_date(pool: PgPool) {
    let mut command = server("stale_schema", "run").await;
    command.env(DB_URL_ENV, database_url(&pool));

    let result = ServerProcess::finished(&mut command).await.unwrap();

    assert!(!result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(!stdout.contains("ready"), "unexpected stdout: {stdout}");
}

#[tokio::test]
async fn server_refuses_to_start_when_database_is_unreachable() {
    let mut command = server("unreachable_db", "run").await;
    command.env(DB_URL_ENV, "postgres://nobody@127.0.0.1:1/none");

    let result = ServerProcess::finished(&mut command).await.unwrap();

    assert!(!result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(!stdout.contains("ready"), "unexpected stdout: {stdout}");
}

#[tokio::test]
async fn server_refuses_to_start_when_database_secret_is_missing() {
    let mut command = server("missing_secret", "run").await;
    let result = ServerProcess::finished(&mut command).await.unwrap();

    assert!(!result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(!stdout.contains("ready"), "unexpected stdout: {stdout}");
}

#[sqlx::test(migrations = false)]
async fn migrate_command_brings_empty_database_up_to_date(pool: PgPool) {
    let mut command = server("migrate", "migrate").await;
    command.env(DB_URL_ENV, database_url(&pool));

    let result = ServerProcess::finished(&mut command).await.unwrap();

    assert!(result.status.success());
    db::check_schema(&pool, &db::GAME_MIGRATOR).await.unwrap();
}
