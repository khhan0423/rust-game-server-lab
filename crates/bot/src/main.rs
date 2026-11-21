use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use infra::logging::{self, ToolConfig};
use infra::shutdown::Shutdown;

#[derive(Debug, Parser)]
struct Cli {
    #[command(flatten)]
    run: bot::RunConfig,
    #[arg(long, default_value = "dev/config/bot.toml")]
    config: PathBuf,
}

fn main() -> anyhow::Result<std::process::ExitCode> {
    let cli = Cli::parse();
    let config: ToolConfig<infra::runtime::ToolRuntime> = infra::config::load(&cli.config)?;
    let _log = logging::init(&config.process.log)?;
    let runtime = config.process.runtime.build()?;
    let summary = runtime.block_on(async {
        let stop = Shutdown::default();
        let token = stop.token();
        let run = bot::run(&cli.run, &token);
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => result,
            signal = Shutdown::signal() => {
                stop.cancel();
                let result = run.await;
                signal.context("listening for shutdown")?;
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
