//! The settings menu: a row of tabs over aligned rows of controls, drawn straight over
//! `&mut Settings`.

use std::ops::RangeInclusive;

use egui::{Align, Layout, RichText, Stroke, Ui};
use strum::{Display, EnumMessage, VariantArray};

use super::theme::{METRICS, PALETTE};
use crate::params::PARAM_SLOTS;
use crate::settings::{Action, CATALOG, Group, ParamSpec, Settings, limits};

const MEGA: f32 = 1e6;
/// The pixel cap a switch turned on starts at, about a 2560x1440 window.
const DEFAULT_CAP_MP: f32 = 3.7;
/// The frame rate a switch turned on starts at.
const DEFAULT_FPS: u32 = 60;

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
    group: ParamGroup,
    /// Where the window's corner was put by dragging its handle; until then, the default.
    pos: Option<egui::Pos2>,
}

/// Where the menu opens.
const MENU_START: egui::Pos2 = egui::pos2(24.0, 24.0);
/// Screens narrower or shorter than these, in points, get the menu filling the screen and
/// laid out for a thumb instead of the floating panel, which would not fit.
const COMPACT_WIDTH: f32 = 700.0;
const COMPACT_HEIGHT: f32 = 520.0;
/// Space left round the menu when it fills the screen.
const COMPACT_MARGIN: f32 = 8.0;
/// Control height when the menu fills the screen, about a fingertip.
const COMPACT_ROW: f32 = 36.0;

/// Whether the screen is too small for the floating panel.
fn compact(ctx: &egui::Context) -> bool {
    let screen = ctx.content_rect();
    screen.width() < COMPACT_WIDTH || screen.height() < COMPACT_HEIGHT
}

/// The parameter groups the Params tab pages between; the debug slot lives under Advanced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Display, VariantArray)]
enum ParamGroup {
    #[default]
    Obstacle,
    Eddies,
    Wake,
    Ball,
    Look,
}

impl ParamGroup {
    const fn group(self) -> Group {
        match self {
            Self::Obstacle => Group::Obstacle,
            Self::Eddies => Group::Eddies,
            Self::Wake => Group::Wake,
            Self::Ball => Group::Ball,
            Self::Look => Group::Look,
        }
    }
}

/// A row of underlined tabs that never wraps, so the window grows to fit it instead.
fn tabs<T: Copy + PartialEq + ToString>(ui: &mut Ui, current: &mut T, all: &[T]) {
    ui.spacing_mut().item_spacing.x = METRICS.tab_gap;
    for &tab in all {
        let selected = *current == tab;
        let text = RichText::new(tab.to_string());
        let text = if selected { text.color(PALETTE.accent) } else { text };
        let response = ui.add(egui::Button::new(text).frame(false));
        if selected {
            let rect = response.rect;
            let y = rect.bottom() + 1.0;
            ui.painter().hline(rect.x_range(), y, Stroke::new(2.0, PALETTE.accent));
        }
        if response.clicked() {
            *current = tab;
        }
    }
}

/// A sliding switch, reported to assistive technology as a checkbox.
fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(1.9 * height, height), egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let state = *on;
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, state, ""));
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let radius = 0.5 * rect.height();
        let track = PALETTE.track.lerp_to_gamma(PALETTE.accent, how_on);
        let edge = if response.hovered() { PALETTE.ink.gamma_multiply(0.5) } else { egui::Color32::TRANSPARENT };
        ui.painter().rect(rect, radius, track, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 0.78 * radius, PALETTE.ink);
    }
    response
}

/// A small drawn cross; true when clicked.
fn close_button(ui: &mut Ui) -> bool {
    let size = egui::vec2(22.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Close"));
    let response = response.on_hover_text("Close the menu");
    let ink = if response.hovered() { PALETTE.ink } else { PALETTE.ink.gamma_multiply(0.6) };
    let r = rect.shrink(5.0);
    let stroke = Stroke::new(2.0, ink);
    ui.painter().line_segment([r.left_top(), r.right_bottom()], stroke);
    ui.painter().line_segment([r.right_top(), r.left_bottom()], stroke);
    response.clicked()
}

/// Two decimals, or three where the whole range is narrower than half a unit.
fn decimals(range: &RangeInclusive<f32>) -> usize {
    if (range.end() - range.start()).abs() < 0.5 { 3 } else { 2 }
}

/// The row's name in the first column, carrying the help text.
fn name(ui: &mut Ui, label: &str, help: &str) -> egui::Id {
    let response = ui.label(label);
    let response = if help.is_empty() { response } else { response.on_hover_text(help) };
    response.id
}

fn float(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, help: &str) -> egui::Response {
    let id = name(ui, label, help);
    let places = decimals(&range);
    ui.add(egui::Slider::new(value, range).fixed_decimals(places)).labelled_by(id)
}

fn whole(ui: &mut Ui, label: &str, value: &mut u32, range: RangeInclusive<u32>, help: &str) {
    let id = name(ui, label, help);
    ui.add(egui::Slider::new(value, range)).labelled_by(id);
    ui.end_row();
}

fn flag(ui: &mut Ui, label: &str, value: &mut bool, help: &str) -> egui::Response {
    if compact(ui.ctx()) {
        return ui
            .horizontal(|ui| {
                let id = name(ui, label, help);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| toggle(ui, value).labelled_by(id)).inner
            })
            .inner;
    }
    let id = name(ui, label, help);
    let response = toggle(ui, value).labelled_by(id);
    ui.end_row();
    response
}

/// A switch for an optional limit and, while it is on, the slider for its value. Switching it
/// on starts the value at `start`; off, the setting has no value at all.
fn limit<T: egui::emath::Numeric>(
    ui: &mut Ui,
    (label, help, value_label): (&str, &str, &str),
    value: &mut Option<T>,
    start: T,
    slider: impl FnOnce(&mut T) -> egui::Slider<'_>,
) -> Option<egui::Response> {
    let mut on = value.is_some();
    if flag(ui, label, &mut on, help).changed() {
        *value = on.then_some(start);
    }
    let v = value.as_mut()?;
    let id = name(ui, value_label, help);
    let response = ui.add(slider(v)).labelled_by(id);
    ui.end_row();
    Some(response)
}

/// Frames per second to hold to, or none to follow the display.
fn frame_rate(ui: &mut Ui, fps: &mut Option<u32>) {
    limit(
        ui,
        ("Limit frame rate", "Hold to a frame rate below the display's, which saves power.", "Frame rate"),
        fps,
        DEFAULT_FPS,
        |v| egui::Slider::new(v, limits::FPS_CAP).suffix(" fps"),
    );
}

fn slider(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, help: &str) {
    float(ui, label, value, range, help);
    ui.end_row();
}

/// Rows of name, control and an optional third column, all lined up; on a small screen each
/// name sits over its control instead, which then takes the full width.
fn rows(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    if compact(ui.ctx()) {
        ui.vertical(|ui| {
            let room = ui.available_width();
            let s = ui.spacing_mut();
            s.slider_width = (room - s.interact_size.x - s.item_spacing.x).max(80.0);
            add(ui);
        });
        return;
    }
    egui::Grid::new(id).num_columns(3).spacing([16.0, 10.0]).show(ui, add);
}

fn param(ui: &mut Ui, spec: &ParamSpec, settings: &mut Settings) {
    let value = settings.params.get_mut(spec.slot);
    let reset = |ui: &mut Ui, value: &mut f32| {
        if *value != 0.0 && ui.small_button("reset").on_hover_text("Back to the designed value").clicked() {
            *value = 0.0;
        }
    };
    if compact(ui.ctx()) {
        let id = ui
            .horizontal(|ui| {
                let id = name(ui, spec.label, spec.help);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| reset(ui, value));
                id
            })
            .inner;
        let places = decimals(&(spec.min..=spec.max));
        ui.add(egui::Slider::new(value, spec.min..=spec.max).fixed_decimals(places)).labelled_by(id);
        return;
    }
    float(ui, spec.label, value, spec.min..=spec.max, spec.help);
    reset(ui, value);
    ui.end_row();
}

fn params(ui: &mut Ui, group: &mut ParamGroup, settings: &mut Settings) {
    if compact(ui.ctx()) {
        egui::ScrollArea::horizontal().id_salt("groups").show(ui, |ui| {
            ui.horizontal(|ui| tabs(ui, group, ParamGroup::VARIANTS));
        });
    } else {
        ui.horizontal(|ui| tabs(ui, group, ParamGroup::VARIANTS));
    }
    ui.add_space(6.0);
    let shown = group.group();
    rows(ui, "params", |ui| {
        for spec in CATALOG.iter().filter(|s| s.group == shown) {
            param(ui, spec, settings);
        }
    });
}

fn render(ui: &mut Ui, settings: &mut Settings, info: &Info) {
    let r = &mut settings.render;
    rows(ui, "render", |ui| {
        flag(ui, "Temporal antialiasing", &mut r.temporal, "Blend each frame with the ones before it.");
        whole(ui, "Supersampling", &mut r.supersampling, limits::SUPERSAMPLING, "Samples per pixel along each axis.");
        whole(ui, "Field step", &mut r.field_step, limits::FIELD_STEP, "Output pixels per flow field texel.");
        frame_rate(ui, &mut r.fps_cap);
        let mut mp = r.max_pixels.map(|p| p / MEGA);
        let range = *limits::MAX_PIXELS.start() / MEGA..=*limits::MAX_PIXELS.end() / MEGA;
        let response = limit(
            ui,
            ("Cap shaded pixels", "Shade fewer pixels than the window has and scale the picture up.", "Max pixels"),
            &mut mp,
            DEFAULT_CAP_MP,
            |v| egui::Slider::new(v, range).logarithmic(true).step_by(0.1).fixed_decimals(1).suffix(" MP"),
        );
        if response.is_some_and(|r| r.changed()) || mp.is_some() != r.max_pixels.is_some() {
            // Whole tenths of a megapixel, which at the top would round past the cap's bound.
            r.max_pixels = mp.map(|mp| {
                ((mp * 10.0).round() / 10.0 * MEGA).clamp(*limits::MAX_PIXELS.start(), *limits::MAX_PIXELS.end())
            });
        }
        let id = name(ui, "Render scale", "Per-axis scale of the shaded picture.");
        let scale = egui::Slider::new(&mut r.render_scale, limits::RENDER_SCALE).step_by(0.05).fixed_decimals(2);
        ui.add(scale).labelled_by(id);
        ui.end_row();
        flag(ui, "Adapt to frame rate", &mut r.adaptive, "Shade fewer pixels while frames run long.");
    });
    ui.separator();
    ui.label(format!("{:.0} fps  |  field {:.1} ms  |  shade {:.1} ms", info.fps, info.field_ms, info.shade_ms));
}

fn motion(ui: &mut Ui, settings: &mut Settings, commands: &mut Vec<Command>) {
    rows(ui, "motion", |ui| {
        slider(ui, "Speed", &mut settings.motion.speed, limits::SPEED, "Flow speed multiplier.");
        let c = &mut settings.camera;
        flag(ui, "Lock the view", &mut c.locked, "Ignore drag, scroll and pinch.");
        slider(ui, "Ease", &mut c.ease_secs, limits::EASE_SECS, "Seconds the view takes to follow input.");
        slider(
            ui,
            "Spring back",
            &mut c.return_secs,
            limits::RETURN_SECS,
            "Seconds a view past the edge takes to return.",
        );
        slider(ui, "Edge give", &mut c.give, limits::GIVE, "How far past the cover the view may be pushed.");
        slider(ui, "Zoom per notch", &mut c.zoom_per_line, limits::ZOOM_PER_LINE, "Zoom factor per scroll line.");
    });
    if ui.button("Reset view").clicked() {
        commands.push(Command::ResetView);
    }
}

fn wallpaper(ui: &mut Ui, settings: &mut Settings) {
    rows(ui, "wallpaper", |ui| {
        let w = &mut settings.wallpaper;
        flag(ui, "Pause when hidden", &mut w.pause_when_hidden, "Stop drawing while the window is covered.");
        flag(ui, "Hint at the menu key", &mut w.hint, "Name the menu key when an unbound key is pressed.");
        flag(
            ui,
            "Hold on the ball opens the menu",
            &mut w.ball_opener,
            "Hold the right button on the ball for a second.",
        );
        frame_rate(ui, &mut settings.render.fps_cap);
    });
}

fn controls(ui: &mut Ui, settings: &Settings, info: &Info, commands: &mut Vec<Command>) {
    let shadowed = settings.keys.shadowed();
    rows(ui, "keys", |ui| {
        for &action in Action::VARIANTS.iter().filter(|a| a.available()) {
            let id = name(ui, action.get_message().unwrap_or_default(), "");
            let text = if info.capturing == Some(action) {
                RichText::new("press a key...").color(PALETTE.accent)
            } else if shadowed.contains(&action) {
                RichText::new(settings.keys.get(action).label()).color(PALETTE.warn)
            } else {
                RichText::new(settings.keys.get(action).label())
            };
            let button = ui.button(text).labelled_by(id);
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
    ui.label(RichText::new("Drag to pan, scroll or pinch to zoom.").color(PALETTE.muted));
}

fn advanced(ui: &mut Ui, settings: &mut Settings) {
    ui.label(RichText::new("Raw shader slots, as offsets on the designed values.").color(PALETTE.muted));
    egui::Grid::new("slots").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
        for slot in 0..PARAM_SLOTS {
            let spec = CATALOG.iter().find(|s| s.slot == slot);
            let label = ui.label(format!("{slot}")).on_hover_text(spec.map_or("unused", |s| s.key));
            // The same bounds the named sliders and a loaded file keep to; the unused slot stays at 0.
            let range = spec.map_or(0.0..=0.0, |s| s.min..=s.max);
            ui.add(egui::DragValue::new(settings.params.get_mut(slot)).range(range).speed(0.01).fixed_decimals(3))
                .labelled_by(label.id);
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
    let note = info.location.as_deref().unwrap_or("Changes last until the page reloads.");
    ui.add(egui::Label::new(RichText::new(note).small().color(PALETTE.muted)).wrap());
}

impl Menu {
    /// Draws the menu when it is open; returns what it asks of the viewer.
    pub(crate) fn show(&mut self, ui: &Ui, settings: &mut Settings, info: &Info) -> Vec<Command> {
        let mut commands = Vec::new();
        if !self.open {
            return commands;
        }
        if compact(ui.ctx()) {
            self.show_compact(ui.ctx(), settings, info, &mut commands);
            return commands;
        }
        // Only the blank stretch of the tab row moves the window, so a drag that slips off a
        // slider never carries the whole menu with it.
        let mut moved = egui::Vec2::ZERO;
        let pos = self.pos.unwrap_or(MENU_START);
        egui::Window::new("Settings")
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .min_width(METRICS.menu_width)
            .max_width(METRICS.menu_width)
            .current_pos(pos)
            .show(ui.ctx(), |ui| {
                // Laid down before the tabs so it sits beneath them: they take clicks, it takes
                // the drags they do not, so a click on a tab still picks it.
                let top = ui.available_rect_before_wrap().min;
                let row = egui::Rect::from_min_max(
                    top,
                    egui::pos2(ui.max_rect().right(), top.y + ui.spacing().interact_size.y),
                );
                let handle = ui.interact(row, ui.scope_id().with("move the menu"), egui::Sense::drag());
                if handle.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    moved = handle.drag_delta();
                }
                ui.horizontal(|ui| {
                    tabs(ui, &mut self.tab, Tab::VARIANTS);
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if close_button(ui) {
                            commands.push(Command::Close);
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ui.ctx().content_rect().height() * 0.6)
                    .show(ui, |ui| self.body(ui, settings, info, &mut commands));
                ui.separator();
                footer(ui, info, &mut commands);
            });
        if moved != egui::Vec2::ZERO {
            let screen = ui.ctx().content_rect();
            let p = pos + moved;
            // Keep enough of the tab row on screen to drag it back.
            self.pos = Some(egui::pos2(
                p.x.clamp(screen.left() - METRICS.menu_width + 80.0, screen.right() - 80.0),
                p.y.clamp(screen.top(), screen.bottom() - 40.0),
            ));
        }
        commands
    }

    fn body(&mut self, ui: &mut Ui, settings: &mut Settings, info: &Info, commands: &mut Vec<Command>) {
        match self.tab {
            Tab::Params => params(ui, &mut self.group, settings),
            Tab::Render => render(ui, settings, info),
            Tab::Motion => motion(ui, settings, commands),
            Tab::Wallpaper => wallpaper(ui, settings),
            Tab::Controls => controls(ui, settings, info, commands),
            Tab::Advanced => advanced(ui, settings),
        }
    }

    /// The menu over the whole of a small screen: tabs that scroll sideways, then one column
    /// of settings with the footer at its end, scrolling as one.
    fn show_compact(&mut self, ctx: &egui::Context, settings: &mut Settings, info: &Info, commands: &mut Vec<Command>) {
        let screen = ctx.content_rect().shrink(COMPACT_MARGIN);
        let frame = egui::Frame::window(&ctx.global_style());
        let inset = frame.total_margin().sum();
        egui::Window::new("Settings")
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .collapsible(false)
            .frame(frame)
            .fixed_pos(screen.min)
            .fixed_size(screen.size() - inset)
            .show(ctx, |ui| {
                let s = ui.spacing_mut();
                s.interact_size.y = COMPACT_ROW;
                s.button_padding.y = 9.0;
                ui.horizontal(|ui| {
                    let close = ui.spacing().interact_size.y;
                    let room = ui.available_width() - close - ui.spacing().item_spacing.x;
                    egui::ScrollArea::horizontal().id_salt("tabs").max_width(room).show(ui, |ui| {
                        ui.horizontal(|ui| tabs(ui, &mut self.tab, Tab::VARIANTS));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if close_button(ui) {
                            commands.push(Command::Close);
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("body").auto_shrink(false).show(ui, |ui| {
                    self.body(ui, settings, info, commands);
                    ui.separator();
                    footer(ui, info, commands);
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use rstest::rstest;

    use super::*;

    struct State {
        menu: Menu,
        settings: Settings,
        commands: Vec<Command>,
    }

    fn harness(tab: Tab, settings: Settings) -> Harness<'static, State> {
        sized(tab, settings, [760.0, 760.0])
    }

    fn sized(tab: Tab, settings: Settings, size: [f32; 2]) -> Harness<'static, State> {
        let state = State { menu: Menu { open: true, tab, ..Menu::default() }, settings, commands: vec![] };
        Harness::builder().with_size(size).build_ui_state(
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
        r.max_pixels = Some(pick(*limits::MAX_PIXELS.start(), *limits::MAX_PIXELS.end()));
        r.fps_cap = Some(end(&limits::FPS_CAP));
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

    /// The row names and the control beside each, in the order the Render tab shows them.
    fn render_rows(settings: &Settings) -> Vec<(&'static str, Role)> {
        let mut rows = vec![
            ("Temporal antialiasing", Role::CheckBox),
            ("Supersampling", Role::Slider),
            ("Field step", Role::Slider),
            ("Limit frame rate", Role::CheckBox),
        ];
        if settings.render.fps_cap.is_some() {
            rows.push(("Frame rate", Role::Slider));
        }
        rows.push(("Cap shaded pixels", Role::CheckBox));
        if settings.render.max_pixels.is_some() {
            rows.push(("Max pixels", Role::Slider));
        }
        rows.extend([("Render scale", Role::Slider), ("Adapt to frame rate", Role::CheckBox)]);
        rows
    }

    #[rstest]
    fn every_row_lines_up_whichever_limits_are_on(
        #[values(None, Some(60))] fps_cap: Option<u32>,
        #[values(None, Some(800_000.0))] max_pixels: Option<f32>,
    ) {
        let mut settings = Settings::default();
        settings.render.fps_cap = fps_cap;
        settings.render.max_pixels = max_pixels;
        let mut harness = harness(Tab::Render, settings.clone());
        harness.run();
        let rows = render_rows(&settings);
        let name_x: Vec<f32> =
            rows.iter().map(|(label, _)| harness.get_by_role_and_label(Role::Label, label).rect().left()).collect();
        let control_x: Vec<f32> =
            rows.iter().map(|(label, role)| harness.get_by_role_and_label(*role, label).rect().left()).collect();
        let tops: Vec<f32> =
            rows.iter().map(|(label, _)| harness.get_by_role_and_label(Role::Label, label).rect().top()).collect();
        check!(name_x.iter().all(|&x| (x - name_x[0]).abs() < 0.5), "names: {name_x:?}");
        check!(control_x.iter().all(|&x| (x - control_x[0]).abs() < 0.5), "controls: {control_x:?}");
        check!(control_x[0] > name_x[0] + 50.0, "controls sit right of the names");
        check!(tops.windows(2).all(|w| w[1] > w[0]), "one row each, top to bottom: {tops:?}");
    }

    fn drag(harness: &mut Harness<'static, State>, from: egui::Pos2, by: egui::Vec2) {
        use egui::{Event, Modifiers, PointerButton};
        let press = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.event(Event::PointerMoved(from));
        harness.event(press(from, true));
        harness.run();
        for step in 1..=10 {
            harness.event(Event::PointerMoved(from + by * (step as f32 / 10.0)));
            harness.run();
        }
        harness.event(press(from + by, false));
        harness.run();
    }

    fn menu_corner(harness: &Harness<'static, State>) -> egui::Pos2 {
        harness.get_by_role_and_label(Role::Button, "Params").rect().min
    }

    #[test]
    fn a_drag_that_slips_off_a_slider_leaves_the_menu_where_it_is() {
        let mut harness = harness(Tab::Render, Settings::default());
        harness.run();
        let before = menu_corner(&harness);
        let rail = harness.get_by_role_and_label(Role::Slider, "Render scale").rect();
        // Just left of the rail, in the gap between a row's name and its control.
        drag(&mut harness, egui::pos2(rail.left() - 6.0, rail.center().y), egui::vec2(-120.0, 80.0));
        check!(menu_corner(&harness) == before);
    }

    #[test]
    fn dragging_the_tab_row_moves_the_menu() {
        let mut harness = harness(Tab::Render, Settings::default());
        harness.run();
        let before = menu_corner(&harness);
        let last_tab = harness.get_by_role_and_label(Role::Button, "Advanced").rect();
        drag(&mut harness, egui::pos2(last_tab.right() + 12.0, last_tab.center().y), egui::vec2(60.0, 90.0));
        let after = menu_corner(&harness);
        check!((after - before - egui::vec2(60.0, 90.0)).length() < 2.0, "{before:?} to {after:?}");
    }

    /// The bounds of every control of `role` showing now, top to bottom.
    fn controls(harness: &Harness<'static, State>, role: Role) -> Vec<egui::Rect> {
        let mut rects: Vec<egui::Rect> = harness.query_all_by_role(role).map(|n| n.rect()).collect();
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
        rects
    }

    #[rstest]
    fn every_control_on_every_tab_answers_and_stays_in_bounds(
        #[values(Tab::Params, Tab::Render, Tab::Motion, Tab::Wallpaper, Tab::Advanced)] tab: Tab,
    ) {
        let mut harness = harness(tab, Settings::default());
        harness.run();
        let corner = menu_corner(&harness);
        // Switching a toggle can add or remove rows, so each pass looks the controls up afresh.
        for i in 0.. {
            let Some(rect) = controls(&harness, Role::CheckBox).get(i).copied() else { break };
            let before = harness.state().settings.clone();
            harness.get_all_by_role(Role::CheckBox).find(|n| n.rect() == rect).expect("still there").click();
            harness.run();
            check!(harness.state().settings != before, "toggle {i} on {tab} did nothing");
            check!(harness.state().settings.clone().clamp().is_empty());
        }
        for i in 0.. {
            let Some(rect) = controls(&harness, Role::Slider).get(i).copied() else { break };
            let before = harness.state().settings.clone();
            let (left, right) =
                (egui::pos2(rect.left() + 1.0, rect.center().y), egui::pos2(rect.right() - 1.0, rect.center().y));
            drag(&mut harness, left, right - left);
            let at_right = harness.state().settings.clone();
            drag(&mut harness, right, left - right);
            let at_left = harness.state().settings.clone();
            check!(at_right != before || at_left != before, "slider {i} on {tab} did nothing");
            check!(at_right.clone().clamp().is_empty(), "slider {i} on {tab} went past its bounds at the right");
            check!(at_left.clone().clamp().is_empty(), "slider {i} on {tab} went past its bounds at the left");
        }
        for i in 0.. {
            let Some(rect) = controls(&harness, Role::SpinButton).get(i).copied() else { break };
            for by in [egui::vec2(600.0, 0.0), egui::vec2(-1200.0, 0.0)] {
                drag(&mut harness, rect.center(), by);
                let issues = harness.state().settings.clone().clamp();
                check!(issues.is_empty(), "value box {i} on {tab} dragged by {by:?}: {issues:?}");
            }
        }
        check!(menu_corner(&harness) == corner, "using the controls moved the menu");
        check!(harness.state().commands.is_empty());
    }

    #[test]
    fn clicking_a_tab_picks_it() {
        let mut harness = harness(Tab::Render, Settings::default());
        harness.run();
        harness.get_by_role_and_label(Role::Button, "Motion").click();
        harness.run();
        check!(harness.state().menu.tab == Tab::Motion);
    }

    #[test]
    fn dragging_from_a_tab_moves_the_menu_and_clicking_one_still_picks_it() {
        let mut harness = harness(Tab::Render, Settings::default());
        harness.run();
        let before = menu_corner(&harness);
        let motion = harness.get_by_role_and_label(Role::Button, "Motion").rect();
        drag(&mut harness, motion.center(), egui::vec2(-10.0, 120.0));
        check!((menu_corner(&harness) - before - egui::vec2(-10.0, 120.0)).length() < 2.0);
        check!(harness.state().menu.tab == Tab::Render, "a drag is not a click");
        harness.get_by_role_and_label(Role::Button, "Motion").click();
        harness.run();
        check!(harness.state().menu.tab == Tab::Motion);
    }

    #[test]
    fn toggling_a_wallpaper_option_edits_the_settings() {
        let mut harness = harness(Tab::Wallpaper, Settings::default());
        harness.get_by_role_and_label(Role::CheckBox, "Hint at the menu key").click();
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
        harness.get_by_role_and_label(Role::CheckBox, "Cap shaded pixels").click();
        harness.run();
        let cap = harness.state().settings.render.max_pixels.expect("switched on") / MEGA;
        check!(((cap * 10.0).round() - cap * 10.0).abs() < 1e-3);
        check!(cap > 0.0);
    }

    /// Names of the tabs, which sit in a row that scrolls sideways and may run off the screen.
    fn is_tab(label: &str) -> bool {
        Tab::VARIANTS
            .iter()
            .map(ToString::to_string)
            .chain(ParamGroup::VARIANTS.iter().map(ToString::to_string))
            .any(|t| t == label)
    }

    #[rstest]
    fn on_a_phone_every_control_fits_the_screen_and_sliders_take_its_width(
        #[values(Tab::Params, Tab::Render, Tab::Motion, Tab::Wallpaper, Tab::Controls, Tab::Advanced)] tab: Tab,
        #[values([390.0, 844.0], [360.0, 640.0], [844.0, 390.0])] size: [f32; 2],
    ) {
        let mut harness = sized(tab, Settings::default(), size);
        harness.run();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
        for role in [Role::Slider, Role::CheckBox, Role::Button, Role::SpinButton] {
            for node in harness.query_all_by_role(role) {
                let label = node.accesskit_node().label().unwrap_or_default();
                if is_tab(&label) {
                    continue;
                }
                let rect = node.rect();
                check!(
                    rect.left() >= screen.left() && rect.right() <= screen.right(),
                    "{role:?} {label:?} at {rect:?}"
                );
            }
        }
        for slider in harness.query_all_by_role(Role::Slider) {
            check!(slider.rect().width() > 0.5 * size[0], "slider only {:?} wide", slider.rect().width());
        }
        let close = harness.get_by_role_and_label(Role::Button, "Close").rect();
        check!(screen.contains_rect(close));
    }

    #[test]
    fn on_a_phone_the_close_button_and_tabs_answer_taps() {
        let mut harness = sized(Tab::Params, Settings::default(), [390.0, 844.0]);
        harness.run();
        harness.get_by_role_and_label(Role::Button, "Render").click();
        harness.run();
        check!(harness.state().menu.tab == Tab::Render);
        harness.get_by_role_and_label(Role::Button, "Close").click();
        harness.run();
        check!(harness.state().commands.contains(&Command::Close));
    }

    #[test]
    fn on_a_phone_a_switch_sits_on_its_label_row() {
        let mut harness = sized(Tab::Wallpaper, Settings::default(), [390.0, 844.0]);
        harness.run();
        let label = harness.get_by_role_and_label(Role::Label, "Pause when hidden").rect();
        let switch = harness.get_by_role_and_label(Role::CheckBox, "Pause when hidden").rect();
        check!((label.center().y - switch.center().y).abs() < 4.0);
        check!(switch.right() > 300.0, "the switch sits at the right edge");
        harness.get_by_role_and_label(Role::CheckBox, "Pause when hidden").click();
        harness.run();
        check!(!harness.state().settings.wallpaper.pause_when_hidden);
    }
}
