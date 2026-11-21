//! Operations flow through both real HTTP servers and a disposable `LoginDB`.

#![expect(clippy::unwrap_used, reason = "test fixture failures fail the test")]

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sqlx::{ConnectOptions, PgPool};
use testkit::{GatewayFixture, ServerProcess, TestDir, WAIT, sibling_binary};
use tokio::process::Command;

const OPS_KEY: &str = "operations-integration-key";

async fn publisher(dir: &TestDir, pool: &PgPool) -> ServerProcess {
    let path = dir.write(
        "publisher.toml",
        &format!(
            "[process]\nname='publisher'\nenvironment='development'\nadmin_addr='127.0.0.1:0'\nlog_root='{}'\nlog_level='info'\n\
         [db]\nmax_connections=2\nacquire_timeout_ms=3000\n\
         [http]\nlisten_addr='127.0.0.1:0'\n\
         [ops_api]\nlisten_addr='127.0.0.1:0'\n\
         [games.lab]\nentry_url='http://127.0.0.1:18200'\n",
            dir.path().join("logs").display(),
        ),
    )
    .await
    .unwrap();
    let command = || {
        let mut command = Command::new(sibling_binary(env!("CARGO_BIN_EXE_ops"), "publisher"));
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
    let migrated = ServerProcess::finished(command().arg("migrate"))
        .await
        .unwrap();
    assert!(migrated.status.success());
    ServerProcess::spawn(command().arg("run")).await.unwrap()
}

async fn start_ops(
    dir: &TestDir,
    services: (&ServerProcess, &ServerProcess),
    environment: &str,
) -> Command {
    let endpoint = services.0.addr("ops_api").unwrap();
    let admin = services.0.addr("admin").unwrap();
    let gateway_admin = services.1.addr("admin").unwrap();
    let path = dir.write(&format!("ops-{environment}.toml"), &format!(
        "[process]\nname='ops'\nenvironment='{environment}'\nadmin_addr='127.0.0.1:0'\nlog_root='{}'\nlog_level='info'\n\
         [http]\nlisten_addr='127.0.0.1:0'\n[operator]\nname='Integration Operator'\n\
         [publisher]\nops_api_url='http://{endpoint}'\n[[servers]]\nname='Publisher'\nadmin_addr='{admin}'\n\
         [[servers]]\nname='Gateway'\nrole='gateway'\nadmin_addr='{gateway_admin}'\n", dir.path().join("logs").display(),
    )).await.unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ops"));
    command
        .arg("--config")
        .arg(path)
        .env("PUBLISHER_OPS_API_KEY", OPS_KEY)
        .env_remove("DATABASE_URL")
        .env_remove("LOGIN_DB_URL")
        .env_remove("GAME_DB_URL")
        .env_remove("POSTGRES_PASSWORD")
        .env_remove("PUBLISHER_GAME_API_KEY");
    command
}

async fn page(client: &Client, url: &str) -> String {
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.text().await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn development_operator_can_sanction_and_audit_a_guest_over_http(pool: PgPool) {
    let dir = TestDir::new(env!("CARGO_TARGET_TMPDIR"), "operations")
        .await
        .unwrap();
    let mut publisher = publisher(&dir, &pool).await;
    let binary = sibling_binary(env!("CARGO_BIN_EXE_ops"), "gateway");
    let mut command = GatewayFixture::new("127.0.0.1:0".parse().unwrap(), 8, 1000)
        .command(&binary, &dir)
        .await
        .unwrap();
    let mut gateway = ServerProcess::spawn(&mut command).await.unwrap();
    let services = (&publisher, &gateway);
    let mut live_command = start_ops(&dir, services, "live").await;
    let mut ops = ServerProcess::spawn(&mut start_ops(&dir, services, "development").await)
        .await
        .unwrap();
    let origin = format!("http://{}", ops.addr("http").unwrap());
    let client = Client::builder().timeout(WAIT).no_proxy().build().unwrap();
    let connections = page(&client, &format!("{origin}/connections")).await;
    assert!(connections.contains("data-gateway=\"Gateway\" data-health=\"up\""));
    assert!(connections.contains("net_connections_current</code></th><td>0</td>"));
    assert!(connections.contains("<th scope=\"row\">auth_timeout</th><td>0</td>"));
    assert!(connections.contains("hx-trigger=\"every 2s\""));
    let servers = page(&client, &format!("{origin}/servers/table")).await;
    assert!(servers.contains("runtime_workers"));
    gateway.stop().await.unwrap();
    let connections = page(&client, &format!("{origin}/connections/table")).await;
    assert!(connections.contains("data-gateway=\"Gateway\" data-health=\"down\""));
    let guest: Value = client
        .post(format!(
            "http://{}/v1/guest/accounts",
            publisher.addr("api").unwrap()
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

    let live = ServerProcess::finished(&mut live_command).await.unwrap();
    assert!(!live.status.success());
    assert_eq!(live.stdout, [0_u8; 0]);
    ops.stop().await.unwrap();
    publisher.stop().await.unwrap();
}
