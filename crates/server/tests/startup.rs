//! Startup barrier of the Game server: it signals ready only after preparation finishes, and does not start if preparation fails.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail the test by panicking"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sqlx::{ConnectOptions, PgPool};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Upper bound for waiting on an event. Not a synchronization mechanism, but a breaker that cuts off a stuck test.
const WAIT: Duration = Duration::from_secs(60);
const DB_URL_ENV: &str = "GAME_DB_URL";

fn write_config(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("server_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("server.toml");
    let contents = format!(
        "[process]\nadmin_addr = \"127.0.0.1:0\"\nlog_dir = '{}'\nlog_level = \"info\"\n\n\
         [db]\nmax_connections = 2\nacquire_timeout_ms = 3000\n",
        dir.join("logs").display()
    );
    std::fs::write(&config, contents).unwrap();
    config
}

fn server(name: &str, command: &str) -> Command {
    let mut process = Command::new(env!("CARGO_BIN_EXE_server"));
    process
        .arg("--config")
        .arg(write_config(name))
        .arg(command)
        .env_remove(DB_URL_ENV)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    process
}

fn database_url(pool: &PgPool) -> String {
    pool.connect_options().to_url_lossy().to_string()
}

async fn ready_admin_addr(child: &mut Child) -> SocketAddr {
    let stdout = child.stdout.take().unwrap();
    let line = tokio::time::timeout(WAIT, BufReader::new(stdout).lines().next_line())
        .await
        .expect("server did not announce readiness in time")
        .unwrap()
        .expect("server exited before announcing readiness");
    line.strip_prefix("ready admin=")
        .expect("unexpected ready line")
        .parse()
        .unwrap()
}

/// Waits for the process to exit and returns (success, standard output).
async fn finished(mut command: Command) -> (bool, String) {
    let output = tokio::time::timeout(WAIT, command.spawn().unwrap().wait_with_output())
        .await
        .expect("server did not exit in time")
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

async fn get(addr: SocketAddr, path: &str) -> reqwest::Response {
    reqwest::get(format!("http://{addr}{path}")).await.unwrap()
}

#[sqlx::test(migrator = "db::GAME_MIGRATOR")]
async fn server_announces_ready_when_schema_is_current(pool: PgPool) {
    let mut child = server("ready", "run")
        .env(DB_URL_ENV, database_url(&pool))
        .spawn()
        .unwrap();

    let admin = ready_admin_addr(&mut child).await;

    assert_eq!(get(admin, "/health/live").await.status(), 200);
    assert_eq!(get(admin, "/health/ready").await.status(), 200);
    let metrics = get(admin, "/metrics").await.text().await.unwrap();
    assert!(
        metrics.lines().any(|line| line == "server_ready 1"),
        "unexpected metrics: {metrics}"
    );
}

#[sqlx::test(migrations = false)]
async fn server_refuses_to_start_when_schema_is_out_of_date(pool: PgPool) {
    let mut command = server("stale_schema", "run");
    command.env(DB_URL_ENV, database_url(&pool));

    let (success, stdout) = finished(command).await;

    assert!(!success);
    assert!(!stdout.contains("ready"), "unexpected stdout: {stdout}");
}

#[tokio::test]
async fn server_refuses_to_start_when_database_is_unreachable() {
    let mut command = server("unreachable_db", "run");
    command.env(DB_URL_ENV, "postgres://nobody@127.0.0.1:1/none");

    let (success, stdout) = finished(command).await;

    assert!(!success);
    assert!(!stdout.contains("ready"), "unexpected stdout: {stdout}");
}

#[tokio::test]
async fn server_refuses_to_start_when_database_secret_is_missing() {
    let (success, stdout) = finished(server("missing_secret", "run")).await;

    assert!(!success);
    assert!(!stdout.contains("ready"), "unexpected stdout: {stdout}");
}

#[sqlx::test(migrations = false)]
async fn migrate_command_brings_empty_database_up_to_date(pool: PgPool) {
    let mut command = server("migrate", "migrate");
    command.env(DB_URL_ENV, database_url(&pool));

    let (success, _stdout) = finished(command).await;

    assert!(success);
    db::check_schema(&pool, &db::GAME_MIGRATOR).await.unwrap();
}
