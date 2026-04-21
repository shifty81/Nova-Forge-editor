//! `atlas_editor_log` — Output Log panel + Bevy→egui tracing bridge.
//!
//! # Tracing bridge
//! Call [`build_editor_log_layer`] inside `LogPlugin::custom_layer` to capture
//! Bevy's `info!`/`warn!`/`error!` (and all other `tracing` events) into the
//! `OutputLog` resource so they appear in the Output Log panel at the bottom of
//! the editor.
//!
//! ```rust,ignore
//! // in atlas_editor_app/src/main.rs:
//! .set(bevy::log::LogPlugin {
//!     custom_layer: atlas_editor_log::build_editor_log_layer,
//!     ..default()
//! })
//! ```

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::log::tracing_subscriber::{self, Layer, registry::LookupSpan};
use bevy_egui::{egui, EguiContexts};
use atlas_editor_core::{EditorMode, EditorPanelOrder, PanelVisibility};

// ────────────────────────────────────────────────────────────────────────────
// Log record
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct LogRecord {
    pub level:   LogLevel,
    pub message: String,
}

// ────────────────────────────────────────────────────────────────────────────
// Shared ring-buffer bridging `tracing` and Bevy ECS
// ────────────────────────────────────────────────────────────────────────────

type SharedBuf = Arc<Mutex<Vec<(LogLevel, String)>>>;

/// Shared suppression counter for noisy Vulkan validation messages.
/// Reports a summary every `VULKAN_SUPPRESS_REPORT_INTERVAL` occurrences so the
/// Output Log doesn't churn on every frame on affected drivers.
type VulkanSuppressState = Arc<Mutex<VulkanSuppressCounter>>;

#[derive(Default)]
struct VulkanSuppressCounter {
    /// Total suppressed since the last summary message was emitted.
    suppressed: u64,
    /// Total suppressed ever (used only for reporting).
    total: u64,
    /// Have we emitted the first "example" line yet?
    seen_example: bool,
}

/// Emit a "(N suppressed)" summary after this many matching events.
const VULKAN_SUPPRESS_REPORT_INTERVAL: u64 = 500;

/// Tracing targets / message markers we collapse.  Match both the target
/// (Vulkan validation layer spam routes through `wgpu_hal::vulkan::instance`
/// on affected drivers) and the specific VUID we know is a false-positive /
/// upstream-fixed issue in Bevy 0.14's wgpu.
const VULKAN_SUPPRESS_MARKERS: &[&str] = &[
    "VUID-vkQueueSubmit-pSignalSemaphores-00067",
];

fn is_vulkan_suppress_target(target: &str) -> bool {
    target.starts_with("wgpu_hal::vulkan") || target.starts_with("wgpu_hal::auxil::dxgi")
}

fn is_vulkan_suppress_message(msg: &str) -> bool {
    VULKAN_SUPPRESS_MARKERS.iter().any(|m| msg.contains(m))
}

/// Bevy resource that holds the bridge buffer.  The `drain_log_bridge` system
/// drains it each frame into [`OutputLog`].
#[derive(Resource, Clone)]
pub struct LogBridge(SharedBuf);

// ────────────────────────────────────────────────────────────────────────────
// tracing Layer that writes into the shared buffer
// ────────────────────────────────────────────────────────────────────────────

pub struct EditorLogLayer {
    buf: SharedBuf,
    vulkan_suppress: VulkanSuppressState,
}

/// Minimal visitor that extracts only the `message` field from a tracing event.
struct MsgVisitor(String);

impl tracing::field::Visit for MsgVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for EditorLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN  => LogLevel::Warning,
            _ => LogLevel::Info,
        };
        let mut visitor = MsgVisitor(String::new());
        event.record(&mut visitor);
        let msg = if visitor.0.is_empty() {
            // Fall-back: include target for non-message events.
            event.metadata().target().to_owned()
        } else {
            visitor.0
        };

        // ── Suppress the Vulkan VUID storm ────────────────────────────────
        // On some drivers wgpu emits this validation error every presented
        // frame; letting it through turns the Output Log panel into a
        // constantly-churning block of red text that reads as UI flicker.
        let target = event.metadata().target();
        if is_vulkan_suppress_target(target) && is_vulkan_suppress_message(&msg) {
            if let Ok(mut state) = self.vulkan_suppress.lock() {
                state.suppressed = state.suppressed.saturating_add(1);
                state.total      = state.total.saturating_add(1);

                // Emit one representative example the first time we see it,
                // then every N occurrences emit a "(N suppressed)" summary.
                let mut summary: Option<(LogLevel, String)> = None;
                if !state.seen_example {
                    state.seen_example = true;
                    summary = Some((
                        LogLevel::Warning,
                        format!(
                            "{msg}  (further occurrences of VUID-…-00067 are suppressed — \
                             driver-level wgpu/Vulkan semaphore-reuse warning, tracked upstream)"
                        ),
                    ));
                    // First example doesn't count toward the next summary.
                    state.suppressed = state.suppressed.saturating_sub(1);
                } else if state.suppressed >= VULKAN_SUPPRESS_REPORT_INTERVAL {
                    let n = state.suppressed;
                    state.suppressed = 0;
                    summary = Some((
                        LogLevel::Info,
                        format!("(suppressed {n} further Vulkan validation messages; total {})", state.total),
                    ));
                }

                if let Some(record) = summary {
                    if let Ok(mut guard) = self.buf.lock() {
                        if guard.len() < 4_000 {
                            guard.push(record);
                        }
                    }
                }
            }
            return;
        }

        if let Ok(mut guard) = self.buf.lock() {
            // Hard cap inside the layer to avoid unbounded growth between drains.
            if guard.len() < 4_000 {
                guard.push((level, msg));
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Public hook for `LogPlugin::custom_layer`
// ────────────────────────────────────────────────────────────────────────────

/// Install the editor log layer and return it for use with
/// `LogPlugin { custom_layer: build_editor_log_layer, ..default() }`.
///
/// The matching [`LogBridge`] resource is inserted into `app` so the
/// `drain_log_bridge` system can read from it each frame.
pub fn build_editor_log_layer(
    app: &mut App,
) -> Option<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync + 'static>> {
    let buf: SharedBuf = Arc::new(Mutex::new(Vec::new()));
    let vulkan_suppress: VulkanSuppressState =
        Arc::new(Mutex::new(VulkanSuppressCounter::default()));
    app.insert_resource(LogBridge(buf.clone()));
    Some(Box::new(EditorLogLayer { buf, vulkan_suppress }))
}

// ────────────────────────────────────────────────────────────────────────────
// Log buffer resource (the panel reads from this)
// ────────────────────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
pub struct OutputLog {
    pub records: Vec<LogRecord>,
    /// Maximum number of records retained.
    pub capacity: usize,
    /// Filter by log level — if true, show records of that level.
    pub show_info:    bool,
    pub show_warning: bool,
    pub show_error:   bool,
}

impl OutputLog {
    pub fn push(&mut self, level: LogLevel, message: impl Into<String>) {
        self.records.push(LogRecord { level, message: message.into() });
        let cap = if self.capacity == 0 { 2_000 } else { self.capacity };
        if self.records.len() > cap {
            self.records.drain(0..self.records.len() - cap);
        }
    }

    pub fn info(&mut self, msg: impl Into<String>)  { self.push(LogLevel::Info,    msg); }
    pub fn warn(&mut self, msg: impl Into<String>)  { self.push(LogLevel::Warning, msg); }
    pub fn error(&mut self, msg: impl Into<String>) { self.push(LogLevel::Error,   msg); }
}

impl Default for LogLevel {
    fn default() -> Self { Self::Info }
}

// ────────────────────────────────────────────────────────────────────────────
// Plugin
// ────────────────────────────────────────────────────────────────────────────

pub struct EditorLogPlugin;

impl Plugin for EditorLogPlugin {
    fn build(&self, app: &mut App) {
        // Ensure OutputLog is present even when the bridge isn't used.
        if !app.world().contains_resource::<OutputLog>() {
            app.insert_resource(OutputLog {
                show_info:    true,
                show_warning: true,
                show_error:   true,
                ..default()
            });
        }
        app
            .add_event::<ClearOutputLog>()
            .add_systems(Update, (drain_log_bridge, handle_clear_log).chain())
            .add_systems(
                Update,
                draw_log_panel.in_set(EditorPanelOrder::BottomLog),
            );
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Events
// ────────────────────────────────────────────────────────────────────────────

/// Request to clear all records from the output log.
#[derive(Event)]
pub struct ClearOutputLog;

// ────────────────────────────────────────────────────────────────────────────
// Systems
// ────────────────────────────────────────────────────────────────────────────

/// Drains the tracing bridge buffer into [`OutputLog`] each frame.
fn drain_log_bridge(bridge: Option<Res<LogBridge>>, mut log: ResMut<OutputLog>) {
    let Some(bridge) = bridge else { return };
    let Ok(mut guard) = bridge.0.lock() else { return };
    for (level, msg) in guard.drain(..) {
        log.push(level, msg);
    }
}

fn handle_clear_log(
    mut events: EventReader<ClearOutputLog>,
    mut log:    ResMut<OutputLog>,
) {
    for _ev in events.read() {
        log.records.clear();
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Panel
// ────────────────────────────────────────────────────────────────────────────

fn draw_log_panel(
    mut contexts: EguiContexts,
    mut log:      ResMut<OutputLog>,
    mut clear_ev: EventWriter<ClearOutputLog>,
    mode:         Res<State<EditorMode>>,
    visibility:   Res<PanelVisibility>,
) {
    if *mode.get() != EditorMode::Editing {
        return;
    }
    if !visibility.output_log {
        return;
    }

    let ctx = contexts.ctx_mut();

    egui::TopBottomPanel::bottom("atlas_output_log")
        .default_height(140.0)
        .resizable(true)
        .show(ctx, |ui| {
            // ── Toolbar ──────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.heading("Output Log");
                ui.separator();
                // Level-filter toggles.
                let info_color = egui::Color32::from_rgb(160, 160, 160);
                let warn_color = egui::Color32::YELLOW;
                let err_color  = egui::Color32::RED;

                let i_text = egui::RichText::new("ℹ Info").color(
                    if log.show_info { info_color } else { egui::Color32::DARK_GRAY },
                );
                if ui.selectable_label(log.show_info, i_text).clicked() {
                    log.show_info = !log.show_info;
                }

                let w_text = egui::RichText::new("⚠ Warn").color(
                    if log.show_warning { warn_color } else { egui::Color32::DARK_GRAY },
                );
                if ui.selectable_label(log.show_warning, w_text).clicked() {
                    log.show_warning = !log.show_warning;
                }

                let e_text = egui::RichText::new("❌ Error").color(
                    if log.show_error { err_color } else { egui::Color32::DARK_GRAY },
                );
                if ui.selectable_label(log.show_error, e_text).clicked() {
                    log.show_error = !log.show_error;
                }

                ui.separator();
                if ui.small_button("Clear").clicked() {
                    clear_ev.send(ClearOutputLog);
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for record in &log.records {
                        let visible = match record.level {
                            LogLevel::Info    => log.show_info,
                            LogLevel::Warning => log.show_warning,
                            LogLevel::Error   => log.show_error,
                        };
                        if !visible { continue; }

                        let (prefix, color) = match record.level {
                            LogLevel::Error   => ("❌ ", egui::Color32::RED),
                            LogLevel::Warning => ("⚠️ ", egui::Color32::YELLOW),
                            LogLevel::Info    => ("ℹ️ ", egui::Color32::from_rgb(160, 160, 160)),
                        };
                        ui.colored_label(color, format!("{prefix}{}", record.message));
                    }
                });
        });
}
