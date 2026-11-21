//! Publisher startup barrier and the guest entry HTTP flow through the real process.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail the test by panicking"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::{ConnectOptions, PgPool};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use uuid::{Uuid, Version};

const WAIT: Duration = Duration::from_secs(60);
const KEY: &str = "publisher-test-key";

async fn write_config(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("publisher_{}_{name}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let config = dir.join("publisher.toml");
    let toml = format!(
        "[process]\nname = 'publisher'\nenvironment = 'development'\nadmin_addr = \"127.0.0.1:0\"\nlog_root = '{}'\nlog_level = \"info\"\n\n\
        [db]\nmax_connections = 2\nacquire_timeout_ms = 3000\n\n\
        [http]\nlisten_addr = \"127.0.0.1:0\"\n\n\
        [ops_api]\nlisten_addr = \"127.0.0.1:0\"\n\n\
        [games.lab]\nentry_url = \"http://127.0.0.1:7200\"\n",
        dir.join("logs").display()
    );
    tokio::fs::write(&config, toml).await.unwrap();
    config
}

async fn publisher(name: &str, pool: &PgPool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_publisher"));
    command
        .arg("run")
        .arg("--config")
        .arg(write_config(name).await)
        .env(
            "LOGIN_DB_URL",
            pool.connect_options().to_url_lossy().as_str(),
        )
        .env("PUBLISHER_GAME_API_KEY", KEY)
        .env("PUBLISHER_OPS_API_KEY", OPS_KEY)
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn ready_addresses(child: &mut Child) -> (SocketAddr, SocketAddr, SocketAddr) {
    let stdout = child.stdout.take().unwrap();
    let line = tokio::time::timeout(WAIT, BufReader::new(stdout).lines().next_line())
        .await
        .expect("publisher did not announce readiness in time")
        .unwrap()
        .expect("publisher exited before announcing readiness");
    let (admin, api) = line
        .strip_prefix("ready admin=")
        .and_then(|fields| fields.split_once(" api="))
        .expect("unexpected ready line");
    let (api, ops) = api.split_once(" ops_api=").unwrap();
    (
        admin.parse().unwrap(),
        api.parse().unwrap(),
        ops.parse().unwrap(),
    )
}

async fn assert_error(response: Response, status: StatusCode, error: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"error": error})
    );
}

fn credential(value: &Value) -> &str {
    let text = value.as_str().unwrap();
    assert_eq!(text.len(), 64);
    assert!(
        text.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    text
}

#[sqlx::test(migrator = "db::login::LOGIN_MIGRATOR")]
async fn publisher_serves_guest_creation_login_and_single_use_verification(pool: PgPool) {
    let mut child = publisher("flow", &pool).await.spawn().unwrap();
    let (admin, api, _) = ready_addresses(&mut child).await;
    let client = Client::builder().timeout(WAIT).build().unwrap();
    let ready = client
        .get(format!("http://{admin}/health/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    let create = format!("http://{api}/v1/guest/accounts");
    let login = format!("http://{api}/v1/guest/login");
    let verify = format!("http://{api}/v1/tickets/verify");

    let response = client.post(&create).json(&json!({})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let guest: Value = response.json().await.unwrap();
    assert!(
        guest.get("account_id").is_none(),
        "guest creation leaked the account ID"
    );
    let guest_id = Uuid::parse_str(guest["guest_id"].as_str().unwrap()).unwrap();
    assert_eq!(guest_id.get_version(), Some(Version::SortRand));
    let secret = credential(&guest["secret"]);
    let login_body = |game_id: &str| json!({"guest_id": guest["guest_id"], "secret": secret, "game_id": game_id});
    for (body, status, code) in [
        (
            json!({"guest_id": guest["guest_id"], "game_id": "lab"}),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (login_body("unknown"), StatusCode::NOT_FOUND, "unknown_game"),
    ] {
        assert_error(
            client.post(&login).json(&body).send().await.unwrap(),
            status,
            code,
        )
        .await;
    }

    let response = client
        .post(&login)
        .json(&login_body("lab"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let entry: Value = response.json().await.unwrap();
    assert!(
        entry.get("account_id").is_none(),
        "guest login leaked the account ID"
    );
    assert_eq!(entry["entry_url"], "http://127.0.0.1:7200");
    assert_eq!(entry["expires_in_secs"], 60);
    let verify_body = json!({"ticket": credential(&entry["ticket"]), "game_id": "lab"});

    for key in [None, Some("publisher-test-kez")] {
        let request = client.post(&verify).json(&verify_body);
        let request = match key {
            None => request,
            Some(key) => request.header("x-api-key", key),
        };
        assert_error(
            request.send().await.unwrap(),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    let verify_request = || {
        client
            .post(&verify)
            .header("x-api-key", KEY)
            .json(&verify_body)
    };
    let response = verify_request().send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let verified: Value = response.json().await.unwrap();
    let account_id = verified["account_id"].as_str().unwrap();
    assert_eq!(
        Uuid::parse_str(account_id).unwrap().get_version(),
        Some(Version::SortRand)
    );
    assert_eq!(
        verified,
        json!({
            "account_id": account_id, "membership": "member", "sanction": "none"
        })
    );
    assert_error(
        verify_request().send().await.unwrap(),
        StatusCode::NOT_FOUND,
        "ticket_invalid",
    )
    .await;
}

#[sqlx::test(migrations = false)]
async fn publisher_refuses_to_start_when_login_schema_is_out_of_date(pool: PgPool) {
    let child = publisher("stale", &pool)
        .await
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let result = tokio::time::timeout(WAIT, child.wait_with_output())
        .await
        .expect("publisher did not exit in time")
        .unwrap();

    assert!(!result.status.success());
    assert_eq!(result.stdout, [0_u8; 0]);
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("schema is out of date")
    );
}

const OPS_KEY: &str = "publisher-operator-test-key";

#[path = "support/operator.rs"]
mod operator;
