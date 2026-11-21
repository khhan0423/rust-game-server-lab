use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use infra::logging::{self, ToolConfig};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Parser)]
struct Cli {
    #[command(flatten)]
    run: bot::RunConfig,
    #[arg(long, default_value = "dev/config/bot.toml")]
    config: PathBuf,
}

fn main() -> anyhow::Result<std::process::ExitCode> {
    let cli = Cli::parse();
    let config: ToolConfig = infra::config::load(&cli.config)?;
    let _log = logging::init(&config.process)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building bot runtime")?;
    let summary = runtime.block_on(async {
        let stop = CancellationToken::new();
        let run = bot::run(&cli.run, &stop);
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => result,
            signal = tokio::signal::ctrl_c() => {
                stop.cancel();
                let result = run.await;
                signal.context("listening for Ctrl+C")?;
                result
            }
        }
    })?;
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &summary)?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(if summary.meets_expectations(&cli.run) {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    })
}
