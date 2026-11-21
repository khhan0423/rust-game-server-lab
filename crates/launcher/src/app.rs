//! Draws the most recent cluster snapshot and sends commands without waiting.

use std::{net::SocketAddr, process::Command as SystemCommand, time::Duration};

use eframe::egui;
use tokio::sync::{mpsc, watch};

use crate::{
    service::{Snapshot, State},
    supervisor::{Command, Operation},
};

#[cfg(test)]
mod tests;

pub(crate) const TITLE: &str = "Rust Game Server Lab - Launcher";

struct LauncherApp {
    commands: mpsc::Sender<Command>,
    updates: watch::Receiver<Vec<Snapshot>>,
    services: Vec<Snapshot>,
    selected: Option<String>,
    feedback: Option<String>,
    pending_input: Vec<egui::Event>,
}

impl LauncherApp {
    fn new(commands: mpsc::Sender<Command>, mut updates: watch::Receiver<Vec<Snapshot>>) -> Self {
        let services = updates.borrow_and_update().clone();
        let selected = services.first().map(|service| service.name().to_owned());
        Self {
            commands,
            updates,
            services,
            selected,
            feedback: None,
            pending_input: Vec::new(),
        }
    }

    fn submit(&mut self, operation: Operation) {
        self.feedback = match operation.submit(&self.commands) {
            Ok(()) => None,
            Err(error) => Some(error.to_owned()),
        };
    }

    fn ops_address(&self) -> Option<SocketAddr> {
        match self
            .services
            .iter()
            .find(|service| service.name() == "ops")?
            .state()
        {
            State::Ready(line) => line.addr("http"),
            State::Stopped
            | State::Starting
            | State::Stopping
            | State::Exited(_)
            | State::Failed(_) => None,
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Start all").clicked() {
                self.submit(Operation::StartAll(None));
            }
            if ui.button("Stop all").clicked() {
                self.submit(Operation::StopAll);
            }
            let address = self.ops_address();
            if ui
                .add_enabled(address.is_some(), egui::Button::new("Open operations tool"))
                .clicked()
                && let Some(address) = address
            {
                let url = format!("http://{address}/");
                #[cfg(target_os = "windows")]
                let mut command = {
                    let mut command = SystemCommand::new("explorer.exe");
                    command.arg(&url);
                    command
                };
                #[cfg(target_os = "macos")]
                let mut command = {
                    let mut command = SystemCommand::new("open");
                    command.arg(&url);
                    command
                };
                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                let mut command = {
                    let mut command = SystemCommand::new("xdg-open");
                    command.arg(&url);
                    command
                };
                match std::thread::Builder::new()
                    .name("open-operations".to_owned())
                    .spawn(move || {
                        if let Err(error) = command.status() {
                            tracing::warn!(%error, "cannot open operations tool");
                        }
                    }) {
                    Ok(_) => self.feedback = None,
                    Err(error) => self.feedback = Some(error.to_string()),
                }
            }
        });
        if let Some(feedback) = &self.feedback {
            ui.label(feedback);
        }
    }

    fn service_table(&mut self, ui: &mut egui::Ui) {
        let mut next = None;
        egui::ScrollArea::both()
            .id_salt("service-table")
            .max_height(250.0)
            .show(ui, |ui| {
                egui::Grid::new("services").striped(true).show(ui, |ui| {
                    for name in ["Service", "State", "PID", "Addresses", "Uptime", "Actions"] {
                        ui.strong(name);
                    }
                    ui.end_row();
                    for service in &self.services {
                        let name = service.name();
                        if ui
                            .selectable_label(self.selected.as_deref() == Some(name), name)
                            .clicked()
                        {
                            self.selected = Some(name.to_owned());
                        }
                        ui.label(service.state().to_string());
                        ui.label(
                            service
                                .pid()
                                .map_or_else(|| "-".to_owned(), |pid| pid.to_string()),
                        );
                        let addresses = match service.state() {
                            State::Ready(line) => line
                                .listeners()
                                .map(|(key, addr)| format!("{key}={addr}"))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            State::Stopped
                            | State::Starting
                            | State::Stopping
                            | State::Exited(_)
                            | State::Failed(_) => "-".to_owned(),
                        };
                        ui.monospace(addresses);
                        ui.label(
                            service.uptime().map_or_else(
                                || "-".to_owned(),
                                |time| format!("{}s", time.as_secs()),
                            ),
                        );
                        ui.horizontal(|ui| {
                            let running = service.pid().is_some();
                            if ui
                                .add_enabled(!running, egui::Button::new("Start"))
                                .clicked()
                            {
                                next = Some(Operation::Start(name.to_owned()));
                            }
                            if ui.add_enabled(running, egui::Button::new("Stop")).clicked() {
                                next = Some(Operation::Stop(name.to_owned()));
                            }
                            if ui.button("Restart").clicked() {
                                next = Some(Operation::Restart(name.to_owned()));
                            }
                            if ui.add_enabled(running, egui::Button::new("Kill")).clicked() {
                                next = Some(Operation::Kill(name.to_owned()));
                            }
                        });
                        ui.end_row();
                    }
                });
            });
        if let Some(operation) = next {
            self.submit(operation);
        }
    }
}

impl eframe::App for LauncherApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.pending_input.append(&mut input.events);
        std::mem::swap(&mut self.pending_input, &mut input.events);
        // egui targets a click using the frame's final pointer position. Keep a
        // later cursor move in the next pass so it cannot retarget the release.
        if let Some(release) = input
            .events
            .iter()
            .position(|event| matches!(event, egui::Event::PointerButton { pressed: false, .. }))
            && input.events[release + 1..].iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerMoved(_)
                        | egui::Event::PointerGone
                        | egui::Event::PointerButton { .. }
                )
            })
        {
            self.pending_input = input.events.split_off(release + 1);
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.updates.has_changed().is_ok_and(|changed| changed) {
            self.services.clone_from(&self.updates.borrow_and_update());
        }
        ui.ctx().request_repaint_after(Duration::from_millis(200));
        self.toolbar(ui);
        self.service_table(ui);
        ui.separator();
        if let Some(service) = self
            .services
            .iter()
            .find(|service| self.selected.as_deref() == Some(service.name()))
        {
            ui.heading(format!("{} output", service.name()));
            egui::ScrollArea::both()
                .id_salt("service-output")
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in service.logs() {
                        ui.monospace(line);
                    }
                });
        }
    }
}

pub(crate) fn run(
    commands: mpsc::Sender<Command>,
    updates: watch::Receiver<Vec<Snapshot>>,
) -> eframe::Result {
    let fallback = infra::fonts::korean_fallback();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |creation| {
            if let Some(bytes) = fallback {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "korean-fallback".into(),
                    egui::FontData::from_owned(bytes).into(),
                );
                for family in fonts.families.values_mut() {
                    family.push("korean-fallback".into());
                }
                creation.egui_ctx.set_fonts(fonts);
            }
            Ok(Box::new(LauncherApp::new(commands, updates)))
        }),
    )
}
