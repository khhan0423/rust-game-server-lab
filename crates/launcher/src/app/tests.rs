use anyhow::Context;
use eframe::App;
use testkit::WAIT;

use crate::{
    session::Session,
    supervisor::tests::{built_binary, fixture, port_released, ready_address},
    topology::Topology,
};

use super::*;

#[test]
fn gui_start_and_stop_all_drive_background_supervisor() -> anyhow::Result<()> {
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let path = caller.block_on(fixture(&built_binary("gateway"), false));
    let (mut session, commands, mut snapshots) = Session::start(Topology::load(&path)?)?;
    let mut app = LauncherApp::new(commands, snapshots.clone());
    let context = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let mut render = |events| {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 650.),
            )),
            events,
            ..Default::default()
        };
        app.raw_input_hook(&context, &mut input);
        context.run_ui(input, |ui| app.ui(ui, &mut frame))
    };
    let layout = render(Vec::new());
    let start = button_position(&layout, "Start all")?;
    let stop = button_position(&layout, "Stop all")?;
    layout.drop_without_applying_deltas();

    render(click_then_move(start)).drop_without_applying_deltas();
    let ready = caller
        .block_on(wait_for_services(&mut snapshots, |state| {
            matches!(state, State::Ready(_))
        }))
        .context("Start all click did not bring both gateways to Ready")?;
    let addresses = ready.iter().map(ready_address).collect::<Vec<_>>();

    render(click_then_move(stop)).drop_without_applying_deltas();
    let stopped = caller
        .block_on(wait_for_services(&mut snapshots, |state| {
            matches!(state, State::Stopped)
        }))
        .context("Stop all click did not bring both gateways to Stopped")?;
    assert!(stopped.iter().all(|service| service.pid().is_none()));
    caller.block_on(async {
        for address in addresses {
            port_released(address).await;
        }
    });
    session.finish()
}

fn button_position(output: &egui::FullOutput, label: &str) -> anyhow::Result<egui::Pos2> {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.text() == label
            {
                Some(text.pos + text.galley.size() / 2.)
            } else {
                None
            }
        })
        .with_context(|| format!("missing toolbar button {label}"))
}

fn click_then_move(pos: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    events.push(egui::Event::PointerMoved(egui::pos2(1000., 600.)));
    events
}

async fn wait_for_services(
    updates: &mut watch::Receiver<Vec<Snapshot>>,
    matches_state: impl Fn(&State) -> bool,
) -> anyhow::Result<Vec<Snapshot>> {
    let current = tokio::time::timeout(
        WAIT,
        updates.wait_for(|services| {
            services.len() == 2
                && (services
                    .iter()
                    .all(|service| matches_state(service.state()))
                    || services
                        .iter()
                        .any(|service| matches!(service.state(), State::Failed(_))))
        }),
    )
    .await??;
    anyhow::ensure!(
        current.iter().all(|service| matches_state(service.state())),
        "unexpected state: {current:?}"
    );
    Ok(current.clone())
}
