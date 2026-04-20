//! In-game HUD: altitude, speed, and nearest planet indicator.
//!
//! The HUD is rendered with Bevy UI text nodes.  Content is only shown when the
//! player is in space (altitude ≥ `ATMOSPHERE_FADE_START`); at lower altitudes
//! the text is hidden so it doesn't clutter the ground-level view.

use bevy::prelude::*;

use crate::components::*;
use crate::config::*;

// ────────────────────────────────────────────────────────────────────────────

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_space_hud)
            .add_systems(Update, update_space_hud);
    }
}

// ────────────────────────────────────────────────────────────────────────────
//  Components
// ────────────────────────────────────────────────────────────────────────────

/// Marks the text node used by the space HUD.
#[derive(Component)]
pub struct SpaceHudText;

// ────────────────────────────────────────────────────────────────────────────
//  Setup
// ────────────────────────────────────────────────────────────────────────────

fn setup_space_hud(mut commands: Commands) {
    // Root node: anchored to the top-right corner.
    commands.spawn(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            top:   Val::Px(12.0),
            right: Val::Px(16.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            ..default()
        },
        ..default()
    })
    .with_children(|parent| {
        parent.spawn((
            TextBundle::from_section(
                "",
                TextStyle {
                    font_size: 16.0,
                    color:     Color::srgba(0.90, 0.95, 1.00, 0.85),
                    ..default()
                },
            ),
            SpaceHudText,
        ));
    });
}

// ────────────────────────────────────────────────────────────────────────────
//  Update
// ────────────────────────────────────────────────────────────────────────────

pub fn update_space_hud(
    player_q:  Query<(&Transform, &PlayerState), With<Player>>,
    bodies_q:  Query<(&Transform, &Name), With<OrbitalBody>>,
    mut hud_q: Query<(&mut Text, &mut Visibility), With<SpaceHudText>>,
) {
    let Ok((mut text, mut vis)) = hud_q.get_single_mut() else { return };

    let Ok((player_tf, player_state)) = player_q.get_single() else {
        *vis = Visibility::Hidden;
        return;
    };

    let player_pos = player_tf.translation;
    let altitude   = player_pos.length() - PLANET_RADIUS;

    // Only show the HUD above the atmosphere fade start.
    if altitude < ATMOSPHERE_FADE_START {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Inherited;

    let speed_ms = player_state.velocity.length();

    // Find the nearest non-sun orbital body.
    let nearest = bodies_q
        .iter()
        .map(|(tf, name)| {
            let d = (tf.translation - player_pos).length();
            (d, name.as_str().to_owned())
        })
        .min_by(|(da, _), (db, _)| da.partial_cmp(db).unwrap_or(std::cmp::Ordering::Equal));

    let nearest_str = if let Some((dist, name)) = nearest {
        let dist_km = dist / 1_000.0;
        if dist_km >= 1_000.0 {
            format!("Nearest: {} ({:.1} Mm)", name, dist_km / 1_000.0)
        } else {
            format!("Nearest: {} ({:.0} km)", name, dist_km)
        }
    } else {
        String::new()
    };

    let alt_km = altitude / 1_000.0;
    let alt_str = if alt_km >= 1_000.0 {
        format!("ALT  {:.2} Mm", alt_km / 1_000.0)
    } else {
        format!("ALT  {:.1} km", alt_km)
    };

    let spd_str = if speed_ms >= 1_000.0 {
        format!("SPD  {:.2} km/s", speed_ms / 1_000.0)
    } else {
        format!("SPD  {:.0} m/s", speed_ms)
    };

    let mut content = format!("{}\n{}", alt_str, spd_str);
    if !nearest_str.is_empty() {
        content.push('\n');
        content.push_str(&nearest_str);
    }

    if let Some(section) = text.sections.first_mut() {
        section.value = content;
    }
}
