//! Starts the desktop test client and wires its UI to the publisher worker.

mod fonts;
mod login;
mod net;
mod profile;
mod ui;
mod world;

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use bevy::prelude::{
    App, ClearColor, Color, DefaultPlugins, IntoScheduleConfigs, PluginGroup, PreStartup, Startup,
    Window, WindowPlugin, default,
};
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass, EguiStartupSet};
use clap::Parser;
use infra::logging::{self, LogLevel};

use crate::profile::ProfileName;

#[derive(Debug, Parser)]
#[command(about = "Desktop test client for the publisher login server")]
struct Args {
    /// Name of the guest credential profile.
    #[arg(long, default_value = "default")]
    profile: ProfileName,
    /// Directory for profiles and operation logs.
    #[arg(long, default_value = "client-data")]
    data_dir: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let _log_guard = logging::init(&args.data_dir.join("logs"), LogLevel::Info)
        .context("start client operation logging")?;
    let profile_path = args.profile.path(&args.data_dir);
    let credentials = profile::load(&profile_path).context("load guest profile")?;
    let (net, worker) = net::start(profile_path.clone()).context("start publisher worker")?;
    tracing::info!(profile = %args.profile, "test client starting");
    let exit = App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Rust Game Server Lab - Test Client".into(),
                resolution: (1100, 720).into(),
                resize_constraints: bevy::window::WindowResizeConstraints {
                    min_width: 650.0,
                    min_height: 480.0,
                    ..default()
                },
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.065, 0.078, 0.098)))
        .insert_resource(fonts::Fallback::load())
        .insert_resource(ui::LoginPanel::new(args.profile, profile_path, credentials))
        .insert_resource(net)
        .add_systems(
            PreStartup,
            world::setup.before(EguiStartupSet::InitContexts),
        )
        .add_systems(Startup, fonts::install)
        .add_systems(EguiPrimaryContextPass, ui::draw)
        .run();
    worker
        .join()
        .map_err(|_| anyhow!("publisher worker panicked"))?;
    if exit.is_error() {
        anyhow::bail!("client window exited with an error");
    }
    tracing::info!("test client stopped");
    Ok(())
}
