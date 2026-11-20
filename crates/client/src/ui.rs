//! Displays the login controls, operation log, and world view message.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};

use crate::login::{Action, Login, Outcome, Reset, ResetControls, State};
use crate::net::{Command, Event, Net};
use crate::profile::{self, Credentials, ProfileName};

/// Editable inputs and display state for the selected profile.
#[derive(Debug, Resource)]
pub(super) struct LoginPanel {
    profile: ProfileName,
    profile_path: PathBuf,
    publisher_url: String,
    game_id: String,
    login: Login,
    log: VecDeque<String>,
    worker_available: bool,
}

impl LoginPanel {
    /// Initializes the login panel from the saved profile, if one exists.
    pub(super) fn new(
        profile: ProfileName,
        profile_path: PathBuf,
        credentials: Option<Credentials>,
    ) -> Self {
        let publisher_url = credentials.as_ref().map_or_else(
            || "http://127.0.0.1:18100".into(),
            |credentials| credentials.publisher_url.clone(),
        );
        Self {
            profile,
            profile_path,
            publisher_url,
            game_id: "lab".into(),
            login: Login::new(credentials),
            log: VecDeque::new(),
            worker_available: true,
        }
    }

    fn submit(&mut self, action: Action, net: &Net) {
        let command = match action {
            Action::Create => Command::Create {
                publisher_url: self.publisher_url.trim().to_owned(),
            },
            Action::LogIn => {
                let Some(credentials) = self.login.credentials() else {
                    return;
                };
                Command::LogIn {
                    credentials: credentials.clone(),
                    game_id: self.game_id.trim().into(),
                }
            }
        };
        match net.commands.try_send(command) {
            Ok(()) => self.login.begin(action),
            Err(TrySendError::Full(_)) => self.record("Error: Request queue is full"),
            Err(TrySendError::Closed(_)) => self.worker_closed(),
        }
    }

    fn worker_closed(&mut self) {
        self.worker_available = false;
        self.record("Error: Publisher worker stopped");
        self.login
            .complete(Outcome::Failed("Publisher worker stopped".into()));
    }

    fn record(&mut self, line: &str) {
        const MAX_LINES: usize = 200;
        if self.log.len() == MAX_LINES {
            self.log.pop_front();
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        let seconds = seconds % 86_400;
        self.log.push_back(format!(
            "{:02}:{:02}:{:02} UTC  {line}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
        ));
    }

    fn draw_login(&mut self, ui: &mut egui::Ui, net: &Net) {
        ui.heading("Login");
        ui.label(format!("Profile: {}", self.profile));
        ui.label(if self.login.is_saved() {
            "Guest credentials saved: Yes"
        } else {
            "Guest credentials saved: No"
        });
        ui.separator();
        ui.label("Publisher URL");
        let buttons = self.login.buttons();
        let url = ui.add_enabled(
            buttons.create,
            egui::TextEdit::singleline(&mut self.publisher_url).desired_width(f32::INFINITY),
        );
        url.on_disabled_hover_text("Saved credentials are tied to their publisher URL.");
        ui.label("Game ID");
        ui.add(egui::TextEdit::singleline(&mut self.game_id).desired_width(f32::INFINITY));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (action, enabled, label) in [
                (Action::Create, buttons.create, "Create guest account"),
                (Action::LogIn, buttons.log_in, "Log in"),
            ] {
                if ui
                    .add_enabled(self.worker_available && enabled, egui::Button::new(label))
                    .clicked()
                {
                    self.submit(action, net);
                }
            }
        });
        self.draw_reset(ui, buttons.reset);
        ui.separator();
        ui.heading("Status");
        let state = match self.login.state() {
            State::NoAccount => "No guest account",
            State::Ready => "Ready to log in",
            State::CreatingAccount => "Creating guest account",
            State::LoggingIn => "Logging in",
            State::TicketHeld(ticket) if ticket.remaining(Instant::now()).is_zero() => {
                "Ticket expired"
            }
            State::TicketHeld(_) => "Ticket received",
            State::Failed(error) => error,
        };
        ui.label(state);
        if let Some(credentials) = self.login.credentials() {
            ui.label(format!("Guest ID: {:.8}", credentials.guest.id));
        } else {
            ui.label("Guest ID: -");
        }
        if let State::TicketHeld(ticket) = self.login.state() {
            let remaining = ticket.remaining(Instant::now());
            let seconds = remaining
                .as_secs()
                .saturating_add(u64::from(remaining.subsec_nanos() != 0));
            ui.label(format!("Ticket: {}...", ticket.prefix));
            ui.label(format!("Expires in: {seconds} s"));
            ui.label(format!("Entry URL: {}", ticket.entry_url));
        } else {
            ui.label("Ticket: -");
            ui.label("Expires in: -");
            ui.label("Entry URL: -");
        }
        ui.separator();
        ui.small("Font check: 한글 가나다");
    }

    fn draw_reset(&mut self, ui: &mut egui::Ui, controls: ResetControls) {
        let enabled = match controls {
            ResetControls::Available => true,
            ResetControls::Disabled => false,
            ResetControls::Confirming => {
                ui.label("This forgets the guest account saved for this profile on this PC. It cannot be recovered.");
                ui.horizontal(|ui| {
                if ui.button("Confirm reset").clicked() {
                    match profile::delete(&self.profile_path) {
                        Ok(()) => {
                            self.login.reset(Reset::Confirmed);
                            self.record("Guest account reset for this profile");
                        }
                        Err(error) => {
                            tracing::warn!(profile = %self.profile, %error, "could not reset guest account");
                            self.record(&format!("Error: Could not reset guest account: {error}"));
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.login.reset(Reset::Cancel);
                }
            });
                return;
            }
        };
        let button = egui::Button::new("Reset account");
        if ui.add_enabled(enabled, button).clicked() {
            self.login.reset(Reset::Request);
        }
    }
}

/// Drains worker events without waiting, then draws the three client panels.
pub(super) fn draw(
    mut contexts: EguiContexts,
    mut panel: ResMut<LoginPanel>,
    mut net: ResMut<Net>,
) -> Result {
    while panel.worker_available {
        match net.events.try_recv() {
            Ok(Event::Sent(path)) => panel.record(&format!("Sent POST {path}")),
            Ok(Event::Received { path, status }) => {
                panel.record(&format!("Received HTTP {status} from {path}"));
            }
            Ok(Event::Finished(outcome)) => {
                if let Outcome::Failed(error) | Outcome::Unsaved { error, .. } = &outcome {
                    panel.record(&format!("Error: {error}"));
                }
                panel.login.complete(outcome);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => panel.worker_closed(),
        }
    }

    let ctx = contexts.ctx_mut()?;
    let mut viewport = egui::Ui::new(
        ctx.clone(),
        "client-viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    egui::Panel::bottom("log")
        .default_size(146.0)
        .show(&mut viewport, |ui| {
            ui.heading("Log");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if panel.log.is_empty() {
                        ui.weak("Publisher activity will appear here.");
                    }
                    for line in &panel.log {
                        ui.monospace(line);
                    }
                });
        });
    egui::Panel::left("login")
        .default_size(300.0)
        .show(&mut viewport, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| panel.draw_login(ui, &net));
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(&mut viewport, |ui| {
            let text = "The world view appears once the client connects\nto a game server.";
            let galley = ui.painter().layout_no_wrap(
                text.into(),
                egui::FontId::proportional(15.0),
                egui::Color32::from_gray(222),
            );
            let rect = egui::Rect::from_center_size(ui.max_rect().center(), galley.size());
            ui.painter().rect_filled(
                rect.expand2(egui::vec2(15.0, 10.0)),
                6.0,
                egui::Color32::from_rgb(25, 31, 41),
            );
            ui.painter().galley(rect.min, galley, egui::Color32::WHITE);
        });
    Ok(())
}
