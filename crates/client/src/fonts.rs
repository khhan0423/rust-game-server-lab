//! Loads the optional Korean font and appends it to egui's default font families.

use std::sync::Arc;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

const FONT_PATH: &str = r"C:\Windows\Fonts\malgun.ttf";
const FONT_NAME: &str = "korean-fallback";

/// Font bytes loaded before the window event loop begins.
#[derive(Resource, Debug)]
pub(super) struct Fallback(Option<Vec<u8>>);

impl Fallback {
    /// Reads the optional font, logging a warning if it cannot be opened.
    pub(super) fn load() -> Self {
        match std::fs::read(FONT_PATH) {
            Ok(bytes) => Self(Some(bytes)),
            Err(error) => {
                tracing::warn!(path = FONT_PATH, %error, "Korean font unavailable");
                Self(None)
            }
        }
    }
}

/// Registers the fallback after the primary egui context is available.
pub(super) fn install(mut contexts: EguiContexts, mut fallback: ResMut<Fallback>) -> Result {
    if let Some(bytes) = fallback.0.take() {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            FONT_NAME.into(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in fonts.families.values_mut() {
            family.push(FONT_NAME.into());
        }
        contexts.ctx_mut()?.set_fonts(fonts);
    }
    Ok(())
}
