//! Entry point for the local server launcher.

mod app;
mod environment;
mod headless;
mod output;
mod service;
mod service_process;
mod session;
mod supervisor;
mod topology;

use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use infra::logging;

use crate::{
    headless::{Mode, write_line},
    session::Session,
    topology::Topology,
};

#[derive(Debug, Parser)]
struct Cli {
    /// Open a terminal mode instead of the launcher window.
    #[arg(long, value_enum)]
    headless: Option<Mode>,
    /// Path to the service topology file.
    #[arg(long, global = true, default_value = "dev/config/launcher.toml")]
    topology: PathBuf,
}

impl Cli {
    fn run(&self) -> anyhow::Result<()> {
        let topology = Topology::load(&self.topology)?;
        let _logging = logging::init(&topology.process().log)?;
        let (mut session, commands, updates) = Session::start(topology)?;
        let result = if let Some(mode) = self.headless {
            session.wait_for(mode.run(&commands))
        } else {
            app::run(commands, updates)
                .map_err(|error| anyhow::anyhow!("opening launcher window: {error}"))
        };
        result.and(session.finish())
    }
}

fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    match (cli.headless, cli.run()) {
        (Some(Mode::Check), Ok(())) => write_line(format_args!("RESULT launcher check PASS"))?,
        (Some(mode), Err(error)) => {
            let reason = format!("{error:#}").replace(['\r', '\n'], " ");
            write_line(format_args!(
                "RESULT launcher {} FAIL: {reason}",
                mode.name()
            ))?;
            return Ok(ExitCode::FAILURE);
        }
        (Some(Mode::Up) | None, result) => result?,
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn explicit_headless_modes_and_default_window_parse() {
        let cases = [
            (vec!["launcher"], None),
            (vec!["launcher", "--headless", "check"], Some(Mode::Check)),
            (vec!["launcher", "--headless", "up"], Some(Mode::Up)),
        ];
        for (args, expected) in cases {
            assert_eq!(Cli::try_parse_from(args).unwrap().headless, expected);
        }
        assert!(Cli::try_parse_from(["launcher", "--headless"]).is_err());
    }
}
