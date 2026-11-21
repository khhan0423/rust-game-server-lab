//! Operations flow through both real HTTP servers and a disposable `LoginDB`.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixture failures fail the test"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sqlx::{ConnectOptions, PgPool};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const WAIT: Duration = Duration::from_secs(60);
const OPS_KEY: &str = "operations-integration-key";

fn publisher_binary() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_ops"))
        .with_file_name(format!("publisher{}", std::env::consts::EXE_SUFFIX))
}

async fn announce(child: &mut Child) -> String {
    let stdout = child.stdout.take().unwrap();
    tokio::time::timeout(WAIT, BufReader::new(stdout).lines().next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("process exited before readiness")
}

fn address(announcement: &str, name: &str) -> SocketAddr {
    assert!(announcement.starts_with("ready "));
    announcement
        .split_ascii_whitespace()
        .filter_map(|field| field.split_once('='))
        .find_map(|(field, addr)| (field == name).then_some(addr))
        .unwrap()
        .parse()
        .unwrap()
}

async fn publisher(dir: &Path, pool: &PgPool) -> Child {
    let path = dir.join("publisher.toml");
    tokio::fs::write(
        &path,
        format!(
            "[process]\nadmin_addr='127.0.0.1:0'\nlog_dir='{}'\nlog_level='info'\n\
         [db]\nmax_connections=2\nacquire_timeout_ms=3000\n\
         [http]\nlisten_addr='127.0.0.1:0'\n\
         [ops_api]\nlisten_addr='127.0.0.1:0'\n\
         [games.lab]\nentry_url='http://127.0.0.1:18200'\n",
            dir.join("publisher").display(),
        ),
    )
    .await
    .unwrap();
    let command = || {
        let mut command = Command::new(publisher_binary());
        command
            .arg("--config")
            .arg(&path)
            .env(
                "LOGIN_DB_URL",
                pool.connect_options().to_url_lossy().as_str(),
            )
            .env("PUBLISHER_GAME_API_KEY", "integration-game-key")
            .env("PUBLISHER_OPS_API_KEY", OPS_KEY)
            .kill_on_drop(true);
        command
    };
    let migrated = tokio::time::timeout(WAIT, command().arg("migrate").status())
        .await
        .unwrap()
        .unwrap();
    assert!(migrated.success());
    command()
        .arg("run")
        .stdout(Stdio::piped())
        .spawn()
        .expect("build publisher before running ops tests")
}

async fn gateway(dir: &Path) -> Child {
    let path = dir.join("gateway.toml");
    tokio::fs::write(
        &path,
        format!(
            "[process]\nadmin_addr='127.0.0.1:0'\nlog_dir='{}'\nlog_level='info'\n\
         [client]\nlisten_addr='127.0.0.1:0'\nmax_connections=8\nauth_timeout_ms=1000\n\
         write_queue_capacity=256\nlog_connections=true\n",
            dir.join("gateway").display(),
        ),
    )
    .await
    .unwrap();
    Command::new(
        Path::new(env!("CARGO_BIN_EXE_ops"))
            .with_file_name(format!("gateway{}", std::env::consts::EXE_SUFFIX)),
    )
    .arg("--config")
    .arg(path)
    .stdout(Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .expect("build gateway before running ops tests")
}

async fn start_ops(dir: &Path, ready: (&str, &str), environment: &str) -> Child {
    let path = dir.join(format!("ops-{environment}.toml"));
    let endpoint = address(ready.0, "ops_api");
    let admin = address(ready.0, "admin");
    let gateway_admin = address(ready.1, "admin");
    tokio::fs::write(&path, format!(
        "environment='{environment}'\n[process]\nadmin_addr='127.0.0.1:0'\nlog_dir='{}'\nlog_level='info'\n\
         [http]\nlisten_addr='127.0.0.1:0'\n[operator]\nname='Integration Operator'\n\
         [publisher]\nops_api_url='http://{endpoint}'\n[[servers]]\nname='Publisher'\nadmin_addr='{admin}'\n\
         [[servers]]\nname='Gateway'\nrole='gateway'\nadmin_addr='{gateway_admin}'\n", dir.join("operations").display(),
    )).await.unwrap();
    Command::new(env!("CARGO_BIN_EXE_ops"))
        .arg("--config")
        .arg(path)
        .env("PUBLISHER_OPS_API_KEY", OPS_KEY)
        .env_remove("DATABASE_URL")
        .env_remove("LOGIN_DB_URL")
        .env_remove("GAME_DB_URL")
        .env_remove("POSTGRES_PASSWORD")
        .env_remove("PUBLISHER_GAME_API_KEY")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

async fn page(client: &Client, url: &str) -> String {
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.text().await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn development_operator_can_sanction_and_audit_a_guest_over_http(pool: PgPool) {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("operations-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let mut publisher = publisher(&dir, &pool).await;
    let ready = announce(&mut publisher).await;
    let mut gateway = gateway(&dir).await;
    let gateway_ready = announce(&mut gateway).await;
    let mut ops = start_ops(&dir, (&ready, &gateway_ready), "development").await;
    let origin = format!("http://{}", address(&announce(&mut ops).await, "http"));
    let client = Client::builder().timeout(WAIT).no_proxy().build().unwrap();
    let connections = page(&client, &format!("{origin}/connections")).await;
    assert!(connections.contains("data-gateway=\"Gateway\" data-health=\"up\""));
    assert!(connections.contains("gateway_connections_current</code></th><td>0</td>"));
    assert!(connections.contains("<th scope=\"row\">auth_timeout</th><td>0</td>"));
    assert!(connections.contains("hx-trigger=\"every 2s\""));
    let servers = page(&client, &format!("{origin}/servers/table")).await;
    assert!(servers.contains("runtime_workers"));
    gateway.kill().await.unwrap();
    let connections = page(&client, &format!("{origin}/connections/table")).await;
    assert!(connections.contains("data-gateway=\"Gateway\" data-health=\"down\""));
    let guest: Value = client
        .post(format!(
            "http://{}/v1/guest/accounts",
            address(&ready, "api")
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let guest_id = guest["guest_id"].as_str().unwrap();
    let accounts = format!("{origin}/accounts?search={guest_id}");
    let html = page(&client, &accounts).await;
    let path = html
        .split_once("href=\"/accounts/")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0;
    let account_id = uuid::Uuid::parse_str(path).unwrap();
    let detail = format!("{origin}/accounts/{account_id}");
    let html = page(&client, &detail).await;
    assert!(html.contains(guest_id));
    assert!(html.contains("data-state=\"active\""));
    let token = html
        .split_once("name=\"csrf\" value=\"")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0;
    let reason = "integration-review";
    let send = |request_origin, token| {
        client
            .post(&detail)
            .header("Origin", request_origin)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("csrf={token}&status=sanctioned&reason={reason}"))
            .send()
    };
    assert_eq!(
        send("http://foreign.test", token).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(origin.as_str(), "wrong-token").await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let changed = send(origin.as_str(), token).await.unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    assert!(
        changed
            .text()
            .await
            .unwrap()
            .contains("data-state=\"sanctioned\"")
    );
    let audit = page(&client, &format!("{origin}/audit")).await;
    assert!(audit.contains("data-action=\"set_status\""));
    assert!(audit.contains(reason));
    assert!(audit.contains(&account_id.to_string()));

    let live = tokio::time::timeout(
        WAIT,
        start_ops(&dir, (&ready, &gateway_ready), "live")
            .await
            .wait_with_output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!live.status.success());
    assert_eq!(live.stdout, [0_u8; 0]);
}
