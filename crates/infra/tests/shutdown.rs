//! The admin shutdown route is exercised against a separate server process.

use std::{io, process::Stdio, time::Duration};

use infra::admin::{STOP, STOP_ENV, STOP_HEADER, StopToken};
use infra::ready::ReadyLine;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::timeout,
};

const CHILD: &str = "INFRA_SHUTDOWN_TEST_CHILD";

#[derive(Debug, thiserror::Error)]
enum ProcessExit {
    #[error(transparent)]
    Startup(#[from] infra::process::ProcessError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[test]
fn serve_in_subprocess() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os(CHILD).is_none() {
        return Ok(());
    }
    let config = toml::from_str(
        "admin_addr='127.0.0.1:0'\nname='stop-test'\nenvironment='development'\nlog_level='info'\nlog_root='../../target/tmp/stop-test'\nworker_threads=1",
    )?;
    infra::server::run::<ProcessExit, _>(&config, async |_| Ok(std::future::ready(())))?;
    Ok(())
}

#[tokio::test]
async fn shutdown_requires_configured_token_and_drains_on_match()
-> Result<(), Box<dyn std::error::Error>> {
    timeout(Duration::from_secs(30), async {
        let client = reqwest::Client::builder().no_proxy().build()?;
        let token = StopToken::generate()?;
        for enabled in [false, true] {
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args(["--exact", "serve_in_subprocess", "--nocapture"])
                .env(CHILD, "1")
                .env_remove(STOP_ENV)
                .kill_on_drop(true)
                .stdout(Stdio::piped());
            if enabled {
                command.env(STOP_ENV, std::str::from_utf8(token.env_value())?);
            }
            let mut child = command.spawn()?;
            let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
            let ready: ReadyLine = loop {
                let line = lines
                    .next_line()
                    .await?
                    .expect("child exited before readiness");
                if let Ok(ready) = line.parse() {
                    break ready;
                }
            };
            let addr = ready.addr("admin").unwrap();
            let response = client
                .post(format!("http://{addr}{STOP}"))
                .header(STOP_HEADER, "wrong-token")
                .send()
                .await?;
            if enabled {
                assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
                token.request(&client, addr).await?;
                assert!(child.wait().await?.success());
            } else {
                assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
                child.kill().await?;
            }
        }
        Ok(())
    })
    .await?
}
