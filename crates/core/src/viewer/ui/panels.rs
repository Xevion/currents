//! The settings menu: one tab per group, drawn straight over `&mut Settings`.

use std::ops::RangeInclusive;

use egui::{RichText, Ui};
use strum::{Display, EnumMessage, VariantArray};

use super::theme::{METRICS, PALETTE};
use crate::params::PARAM_SLOTS;
use crate::settings::{Action, CATALOG, Group, ParamSpec, Settings, limits};

const MEGA: f32 = 1e6;
/// The pixel cap a freshly ticked box starts at, about a 2560x1440 window.
const DEFAULT_CAP_MP: f32 = 3.7;
const MIN_CAP_MP: f32 = 0.1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Display, VariantArray)]
pub(crate) enum Tab {
    #[default]
    Params,
    Render,
    Motion,
    Wallpaper,
    Controls,
    Advanced,
}

/// What the menu asks the viewer to do beyond changing settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Close,
    ResetView,
    ResetDefaults,
    OpenFile,
    CopyPath,
    /// Wait for the next key press and bind it to the action.
    Capture(Action),
}

/// What the menu shows but does not own.
#[derive(Default)]
pub(crate) struct Info {
    /// Frames per second over the last moment, and the field and shade pass times.
    pub(crate) fps: f32,
    pub(crate) field_ms: f32,
    pub(crate) shade_ms: f32,
    /// Where settings are saved, when they are.
    pub(crate) location: Option<String>,
    /// Why settings are not being saved, when they are not.
    pub(crate) problem: Option<String>,
    /// Problems the settings file had when it was read.
    pub(crate) issues: Vec<String>,
    /// The action waiting for its new key.
    pub(crate) capturing: Option<Action>,
}

#[derive(Default)]
pub(crate) struct Menu {
    pub(crate) open: bool,
    tab: Tab,
}

fn help(response: &egui::Response, text: &str) {
    if !text.is_empty() {
        response.clone().on_hover_text(text);
    }
}

fn float(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, text: &str) {
    help(&ui.add(egui::Slider::new(value, range).text(label)), text);
}

fn whole(ui: &mut Ui, label: &str, value: &mut u32, range: RangeInclusive<u32>, text: &str) {
    help(&ui.add(egui::Slider::new(value, range).text(label)), text);
}

fn flag(ui: &mut Ui, label: &str, value: &mut bool, text: &str) {
    help(&ui.checkbox(value, label), text);
}

fn param(ui: &mut Ui, spec: &ParamSpec, settings: &mut Settings) {
    ui.horizontal(|ui| {
        let value = settings.params.get_mut(spec.slot);
        float(ui, spec.label, value, spec.min..=spec.max, spec.help);
        if *value != 0.0 && ui.small_button("reset").on_hover_text("Back to the designed value").clicked() {
            *value = 0.0;
        }
    });
}

fn params(ui: &mut Ui, settings: &mut Settings) {
    for &group in Group::VARIANTS.iter().filter(|&&g| g != Group::Debug) {
        egui::CollapsingHeader::new(group.to_string()).default_open(group == Group::Obstacle).show(ui, |ui| {
            for spec in CATALOG.iter().filter(|s| s.group == group) {
                param(ui, spec, settings);
            }
        });
    }
}

fn render(ui: &mut Ui, settings: &mut Settings, info: &Info) {
    let r = &mut settings.render;
    flag(ui, "Temporal antialiasing", &mut r.temporal, "Blend each frame with the ones before it.");
    whole(ui, "Supersampling", &mut r.supersampling, limits::SUPERSAMPLING, "Samples per pixel along each axis.");
    whole(ui, "Field step", &mut r.field_step, limits::FIELD_STEP, "Output pixels per flow field texel.");
    whole(ui, "FPS cap", &mut r.fps_cap, limits::FPS_CAP, "Frames per second; 0 follows the display.");
    ui.separator();
    let mut capped = r.max_pixels > 0.0;
    if ui.checkbox(&mut capped, "Cap shaded pixels").changed() {
        r.max_pixels = if capped { DEFAULT_CAP_MP * MEGA } else { 0.0 };
    }
    if capped {
        let mut mp = r.max_pixels / MEGA;
        let slider = egui::Slider::new(&mut mp, MIN_CAP_MP..=*limits::MAX_PIXELS.end() / MEGA)
            .logarithmic(true)
            .step_by(0.1)
            .suffix(" MP")
            .text("Max pixels");
        let response = ui.add(slider);
        help(&response, "Millions of pixels shaded per frame.");
        if response.changed() {
            r.max_pixels = (mp * 10.0).round() / 10.0 * MEGA;
        }
    }
    help(
        &ui.add(egui::Slider::new(&mut r.render_scale, limits::RENDER_SCALE).step_by(0.05).text("Render scale")),
        "Per-axis scale of the shaded picture.",
    );
    flag(ui, "Adapt to frame rate", &mut r.adaptive, "Shade fewer pixels while frames run long.");
    ui.separator();
    ui.label(format!("{:.0} fps  |  field {:.1} ms  |  shade {:.1} ms", info.fps, info.field_ms, info.shade_ms));
}

fn motion(ui: &mut Ui, settings: &mut Settings, commands: &mut Vec<Command>) {
    float(ui, "Speed", &mut settings.motion.speed, limits::SPEED, "Flow speed multiplier.");
    ui.separator();
    let c = &mut settings.camera;
    flag(ui, "Lock the view", &mut c.locked, "Ignore drag, scroll and pinch.");
    float(ui, "Ease", &mut c.ease_secs, limits::EASE_SECS, "Seconds the view takes to follow input.");
    float(ui, "Spring back", &mut c.return_secs, limits::RETURN_SECS, "Seconds a view past the edge takes to return.");
    float(ui, "Edge give", &mut c.give, limits::GIVE, "How far past the cover the view may be pushed.");
    float(ui, "Zoom per notch", &mut c.zoom_per_line, limits::ZOOM_PER_LINE, "Zoom factor per scroll line.");
    if ui.button("Reset view").clicked() {
        commands.push(Command::ResetView);
    }
}

fn wallpaper(ui: &mut Ui, settings: &mut Settings) {
    let w = &mut settings.wallpaper;
    flag(ui, "Pause when hidden", &mut w.pause_when_hidden, "Stop drawing while the window is covered.");
    flag(ui, "Hint at the menu key", &mut w.hint, "Name the menu key when an unbound key is pressed.");
    flag(ui, "Hold on the ball opens the menu", &mut w.ball_opener, "Hold the right button on the ball for a second.");
    whole(ui, "FPS cap", &mut settings.render.fps_cap, limits::FPS_CAP, "Frames per second; 0 follows the display.");
}

fn controls(ui: &mut Ui, settings: &Settings, info: &Info, commands: &mut Vec<Command>) {
    let shadowed = settings.keys.shadowed();
    egui::Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
        for &action in Action::VARIANTS.iter().filter(|a| a.available()) {
            ui.label(action.get_message().unwrap_or_default());
            let text = if info.capturing == Some(action) {
                RichText::new("press a key...").color(PALETTE.accent)
            } else if shadowed.contains(&action) {
                RichText::new(settings.keys.get(action).label()).color(PALETTE.warn)
            } else {
                RichText::new(settings.keys.get(action).label())
            };
            let button = ui.button(text);
            let button = if shadowed.contains(&action) {
                button.on_hover_text("Another action already uses this key")
            } else {
                button.on_hover_text("Click, then press the new key; Esc cancels")
            };
            if button.clicked() {
                commands.push(Command::Capture(action));
            }
            ui.end_row();
        }
    });
    ui.label("Drag to pan, scroll or pinch to zoom.");
}

fn advanced(ui: &mut Ui, settings: &mut Settings) {
    ui.label("Raw shader slots, as offsets on the designed values.");
    egui::Grid::new("slots").num_columns(4).show(ui, |ui| {
        for slot in 0..PARAM_SLOTS {
            let name = CATALOG.iter().find(|s| s.slot == slot).map_or("unused", |s| s.key);
            let label = ui.label(format!("{slot}")).on_hover_text(name);
            ui.add(egui::DragValue::new(settings.params.get_mut(slot)).speed(0.01)).labelled_by(label.id);
            if slot % 2 == 1 {
                ui.end_row();
            }
        }
    });
}

fn footer(ui: &mut Ui, info: &Info, commands: &mut Vec<Command>) {
    for issue in &info.issues {
        ui.label(RichText::new(issue).color(PALETTE.warn).small());
    }
    if let Some(problem) = &info.problem {
        ui.label(RichText::new(problem).color(PALETTE.warn));
    }
    ui.horizontal(|ui| {
        if ui.button("Reset to defaults").clicked() {
            commands.push(Command::ResetDefaults);
        }
        if info.location.is_some() {
            if ui.button("Open TOML").clicked() {
                commands.push(Command::OpenFile);
            }
            if ui.button("Copy TOML path").clicked() {
                commands.push(Command::CopyPath);
            }
        }
    });
    match &info.location {
        Some(path) => ui.label(RichText::new(path).small().weak()),
        None => ui.label(RichText::new("Changes last until the page reloads.").small().weak()),
    };
}

impl Menu {
    /// Draws the menu when it is open; returns what it asks of the viewer.
    pub(crate) fn show(&mut self, ui: &Ui, settings: &mut Settings, info: &Info) -> Vec<Command> {
        let mut commands = Vec::new();
        if !self.open {
            return commands;
        }
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(METRICS.menu_width)
            .default_pos([24.0, 24.0])
            .show(ui.ctx(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &tab in Tab::VARIANTS {
                        ui.selectable_value(&mut self.tab, tab, tab.to_string());
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.6).show(
                    ui,
                    |ui| match self.tab {
                        Tab::Params => params(ui, settings),
                        Tab::Render => render(ui, settings, info),
                        Tab::Motion => motion(ui, settings, &mut commands),
                        Tab::Wallpaper => wallpaper(ui, settings),
                        Tab::Controls => controls(ui, settings, info, &mut commands),
                        Tab::Advanced => advanced(ui, settings),
                    },
                );
                ui.separator();
                footer(ui, info, &mut commands);
            });
        if !open {
            commands.push(Command::Close);
        }
        commands
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use rstest::rstest;

    use super::*;

    struct State {
        menu: Menu,
        settings: Settings,
        commands: Vec<Command>,
    }

    fn harness(tab: Tab, settings: Settings) -> Harness<'static, State> {
        let state = State { menu: Menu { open: true, tab }, settings, commands: vec![] };
        Harness::builder().with_size([520.0, 720.0]).build_ui_state(
            |ui, state: &mut State| {
                super::super::theme::install(ui.ctx());
                let commands = state.menu.show(ui, &mut state.settings, &Info::default());
                state.commands.extend(commands);
            },
            state,
        )
    }

    #[derive(Clone, Copy, Debug)]
    enum Edge {
        Low,
        Default,
        High,
    }

    /// Every setting the menu shows at one end of what it offers.
    fn at(edge: Edge) -> Settings {
        let mut s = Settings::default();
        let pick = |low: f32, high: f32| match edge {
            Edge::Low => low,
            Edge::High => high,
            Edge::Default => unreachable!(),
        };
        if matches!(edge, Edge::Default) {
            return s;
        }
        let end = |r: &RangeInclusive<u32>| if matches!(edge, Edge::Low) { *r.start() } else { *r.end() };
        let r = &mut s.render;
        r.supersampling = end(&limits::SUPERSAMPLING);
        r.field_step = end(&limits::FIELD_STEP);
        r.fps_cap = end(&limits::FPS_CAP);
        r.max_pixels = pick(MIN_CAP_MP * MEGA, *limits::MAX_PIXELS.end());
        r.render_scale = pick(*limits::RENDER_SCALE.start(), *limits::RENDER_SCALE.end());
        r.adaptive = matches!(edge, Edge::High);
        r.temporal = matches!(edge, Edge::High);
        s.motion.speed = pick(*limits::SPEED.start(), *limits::SPEED.end());
        let c = &mut s.camera;
        c.locked = matches!(edge, Edge::High);
        c.ease_secs = pick(*limits::EASE_SECS.start(), *limits::EASE_SECS.end());
        c.return_secs = pick(*limits::RETURN_SECS.start(), *limits::RETURN_SECS.end());
        c.give = pick(*limits::GIVE.start(), *limits::GIVE.end());
        c.zoom_per_line = pick(*limits::ZOOM_PER_LINE.start(), *limits::ZOOM_PER_LINE.end());
        for spec in &CATALOG {
            *s.params.get_mut(spec.slot) = pick(spec.min, spec.max);
        }
        s
    }

    #[rstest]
    fn every_tab_shows_any_settings_without_changing_them(
        #[values(Tab::Params, Tab::Render, Tab::Motion, Tab::Wallpaper, Tab::Controls, Tab::Advanced)] tab: Tab,
        #[values(Edge::Low, Edge::Default, Edge::High)] edge: Edge,
    ) {
        let settings = at(edge);
        check!(settings.clone().clamp().is_empty());
        let mut harness = harness(tab, settings.clone());
        harness.run();
        check!(harness.state().settings == settings);
        check!(harness.state().commands.is_empty());
    }

    #[test]
    fn toggling_a_wallpaper_option_edits_the_settings() {
        let mut harness = harness(Tab::Wallpaper, Settings::default());
        harness.get_by_label("Hint at the menu key").click();
        harness.run();
        check!(!harness.state().settings.wallpaper.hint);
    }

    #[test]
    fn the_reset_button_asks_for_the_defaults() {
        let mut harness = harness(Tab::Wallpaper, Settings::default());
        harness.get_by_label("Reset to defaults").click();
        harness.run();
        check!(harness.state().commands.contains(&Command::ResetDefaults));
    }

    #[test]
    fn ticking_the_pixel_cap_starts_it_on_a_whole_tenth_of_a_megapixel() {
        let mut harness = harness(Tab::Render, Settings::default());
        harness.get_by_label("Cap shaded pixels").click();
        harness.run();
        let cap = harness.state().settings.render.max_pixels / MEGA;
        check!(((cap * 10.0).round() - cap * 10.0).abs() < 1e-3);
        check!(cap > 0.0);
    }
}
