//! Draws the grid behind the world view overlay.

use bevy::prelude::*;

const STEP: f32 = 36.0;
const SPAN: f32 = 4096.0;

/// Spawns the world camera and grid lines under the egui panels.
pub(super) fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
    for index in -56i16..=56 {
        let position = STEP * f32::from(index);
        let color = if index % 5 == 0 {
            Color::srgb(0.15, 0.19, 0.25)
        } else {
            Color::srgb(0.105, 0.131, 0.171)
        };
        commands.spawn((
            Sprite::from_color(color, Vec2::new(1.0, SPAN)),
            Transform::from_xyz(position, 0.0, 0.0),
        ));
        commands.spawn((
            Sprite::from_color(color, Vec2::new(SPAN, 1.0)),
            Transform::from_xyz(0.0, position, 0.0),
        ));
    }
}
