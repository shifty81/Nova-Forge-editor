//! `nf_editor_core` — editor modes, panel contracts, docking layout, shared events.

use bevy::prelude::*;
use nf_commands::{EditorCommand, EditorCommandContext};
use nf_selection::{FocusedEntity, SelectionChanged, SelectedEntities};

// ────────────────────────────────────────────────────────────────────────────
// Shared entity metadata
// ────────────────────────────────────────────────────────────────────────────

/// Display name shown in the outliner and details panel for an entity.
/// Add this component to any entity that should be visible and named in the editor.
#[derive(Component, Default, Clone)]
pub struct EntityLabel(pub String);

/// Marks the camera used by the editor viewport (not the runtime/game camera).
/// The PIE system deactivates this camera when Play starts and reactivates it
/// when Play stops.
#[derive(Component)]
pub struct EditorCamera;

// ────────────────────────────────────────────────────────────────────────────
// Editor mode state machine
// ────────────────────────────────────────────────────────────────────────────

/// Top-level editor state.  Systems use this to decide whether to run.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EditorMode {
    /// Editor UI is active; scene entities are editable; game systems paused.
    #[default]
    Editing,
    /// Gameplay systems are running; input routed to the runtime world.
    PlayingInEditor,
    /// Gameplay/physics runs but the editor camera stays detached.
    Simulating,
    /// Runtime is frozen; frame stepping is available.
    Paused,
}

// ────────────────────────────────────────────────────────────────────────────
// Panel IDs — used for docking layout and focus routing
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Viewport,
    Outliner,
    Details,
    ContentBrowser,
    OutputLog,
    Scene,
}

// ────────────────────────────────────────────────────────────────────────────
// Shared editor events
// ────────────────────────────────────────────────────────────────────────────

/// Request the editor to enter a different mode.
#[derive(Event, Debug)]
pub struct RequestEditorMode(pub EditorMode);

/// Request a full redraw of all editor panels next frame.
#[derive(Event, Debug)]
pub struct RefreshPanels;

// ────────────────────────────────────────────────────────────────────────────
// Entity spawning — Create menu
// ────────────────────────────────────────────────────────────────────────────

/// The kind of primitive entity to create from the Create menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveKind {
    /// An empty entity with only a Transform.
    Blank,
    /// A 1 m³ unit cube mesh.
    Cube,
    /// A unit sphere (UV subdivided).
    Sphere,
    /// A 5 m × 5 m horizontal plane.
    Plane,
    /// A directional light (like a sun).
    DirectionalLight,
    /// A point light.
    PointLight,
}

impl PrimitiveKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Blank           => "Entity",
            Self::Cube            => "Cube",
            Self::Sphere          => "Sphere",
            Self::Plane           => "Plane",
            Self::DirectionalLight => "Directional Light",
            Self::PointLight      => "Point Light",
        }
    }
}

/// Send this event to spawn a new entity of the given kind at the world origin.
/// The new entity becomes `FocusedEntity` immediately.
#[derive(Event, Debug, Clone, Copy)]
pub struct SpawnEntityRequest(pub PrimitiveKind);

// ────────────────────────────────────────────────────────────────────────────
// Plugin
// ────────────────────────────────────────────────────────────────────────────

pub struct EditorCorePlugin;

impl Plugin for EditorCorePlugin {
    fn build(&self, app: &mut App) {
        app
            .init_state::<EditorMode>()
            .add_event::<RequestEditorMode>()
            .add_event::<RefreshPanels>()
            .add_event::<SpawnEntityRequest>()
            .add_systems(Update, (handle_mode_requests, handle_spawn_entity).chain());
    }
}

fn handle_mode_requests(
    mut events: EventReader<RequestEditorMode>,
    mut next:   ResMut<NextState<EditorMode>>,
) {
    for ev in events.read() {
        next.set(ev.0);
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Entity spawn handler
// ────────────────────────────────────────────────────────────────────────────

fn handle_spawn_entity(
    mut events:    EventReader<SpawnEntityRequest>,
    mut commands:  Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut focused:   ResMut<FocusedEntity>,
    mut selected:  ResMut<SelectedEntities>,
    mut changed:   EventWriter<SelectionChanged>,
) {
    for ev in events.read() {
        let kind  = ev.0;
        let label = EntityLabel(kind.label().into());

        let entity = match kind {
            PrimitiveKind::Blank => {
                commands.spawn((
                    TransformBundle::default(),
                    VisibilityBundle::default(),
                    label,
                )).id()
            }

            PrimitiveKind::Cube => {
                let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
                let mat  = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.7, 0.7, 0.72),
                    perceptual_roughness: 0.8,
                    ..default()
                });
                commands.spawn((
                    PbrBundle { mesh, material: mat, ..default() },
                    label,
                )).id()
            }

            PrimitiveKind::Sphere => {
                let mesh = meshes.add(Sphere::new(0.5).mesh().uv(32, 18));
                let mat  = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.7, 0.7, 0.72),
                    perceptual_roughness: 0.8,
                    ..default()
                });
                commands.spawn((
                    PbrBundle { mesh, material: mat, ..default() },
                    label,
                )).id()
            }

            PrimitiveKind::Plane => {
                let mesh = meshes.add(Plane3d::default().mesh().size(5.0, 5.0));
                let mat  = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.5, 0.5, 0.5),
                    perceptual_roughness: 1.0,
                    ..default()
                });
                commands.spawn((
                    PbrBundle { mesh, material: mat, ..default() },
                    label,
                )).id()
            }

            PrimitiveKind::DirectionalLight => {
                commands.spawn((
                    DirectionalLightBundle {
                        directional_light: DirectionalLight {
                            illuminance: 10_000.0,
                            shadows_enabled: true,
                            ..default()
                        },
                        transform: Transform::from_rotation(Quat::from_euler(
                            EulerRot::YXZ, 0.0, -std::f32::consts::FRAC_PI_4, 0.0,
                        )),
                        ..default()
                    },
                    label,
                )).id()
            }

            PrimitiveKind::PointLight => {
                commands.spawn((
                    PointLightBundle {
                        point_light: PointLight {
                            intensity: 800.0,
                            radius: 0.1,
                            shadows_enabled: true,
                            ..default()
                        },
                        ..default()
                    },
                    label,
                )).id()
            }
        };

        // Select the newly created entity.
        focused.0 = Some(entity);
        selected.set_single(entity);
        changed.send(SelectionChanged);
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Concrete editor commands defined here (has access to EntityLabel)
// ────────────────────────────────────────────────────────────────────────────

/// Rename an entity's [`EntityLabel`] — supports undo/redo.
pub struct RenameEntityCommand {
    pub entity:   Entity,
    pub old_name: String,
    pub new_name: String,
}

impl EditorCommand for RenameEntityCommand {
    fn apply(&mut self, ctx: &mut EditorCommandContext) {
        if let Some(mut lbl) = ctx.world.get_mut::<EntityLabel>(self.entity) {
            lbl.0 = self.new_name.clone();
        }
    }
    fn undo(&mut self, ctx: &mut EditorCommandContext) {
        if let Some(mut lbl) = ctx.world.get_mut::<EntityLabel>(self.entity) {
            lbl.0 = self.old_name.clone();
        }
    }
    fn label(&self) -> &str { "Rename" }
}

