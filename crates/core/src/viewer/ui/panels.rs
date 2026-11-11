//! The settings menu: a row of tabs over aligned rows of controls, drawn straight over
//! `&mut Settings`.

use std::ops::RangeInclusive;

use egui::{Align, Layout, RichText, Stroke, Ui};
use strum::{Display, EnumMessage, VariantArray};

use super::theme::{METRICS, PALETTE};
use crate::params::PARAM_SLOTS;
use crate::renderer::PassTimes;
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
    /// Share the settings as a link, through the page.
    CopyLink,
    /// Wait for the next key press and bind it to the action.
    Capture(Action),
}

/// What frames are shaded at, which an adaptive budget moves while frames run long.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shading {
    /// Pixels shaded per frame, before supersampling.
    pub(crate) size: [u32; 2],
    /// The window those pixels are scaled up to fill.
    pub(crate) window: [u32; 2],
    pub(crate) field_step: u32,
    pub(crate) supersampling: u32,
    /// Whether frames running long took the scale or the field below the settings.
    pub(crate) lowered: bool,
}

/// What the menu shows but does not own.
#[derive(Default)]
pub(crate) struct Info {
    /// Frames per second over the last moment.
    pub(crate) fps: f32,
    /// GPU time in each pass of a recent frame; `None` where the device does not time them.
    pub(crate) passes: Option<PassTimes>,
    /// What frames are shaded at now, once the viewer draws.
    pub(crate) shading: Option<Shading>,
    /// Where settings are saved, when they are.
    pub(crate) location: Option<String>,
    /// Why settings are not being saved, when they are not.
    pub(crate) problem: Option<String>,
    /// Problems the starting settings had, from the settings file or the page's link.
    pub(crate) issues: Vec<String>,
    /// The action waiting for its new key.
    pub(crate) capturing: Option<Action>,
    /// Whether the page can share the settings as a link.
    pub(crate) can_share: bool,
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
    ui.spacing_mut().item_spacing.x = if compact(ui.ctx()) { METRICS.tab_gap * 0.7 } else { METRICS.tab_gap };
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

/// How a slider maps and shows its value.
struct Spec<T> {
    range: RangeInclusive<T>,
    log: bool,
    step: Option<f64>,
    decimals: Option<usize>,
    suffix: &'static str,
}

impl<T: egui::emath::Numeric> Spec<T> {
    fn new(range: RangeInclusive<T>) -> Self {
        Self { range, log: false, step: None, decimals: None, suffix: "" }
    }

    fn widget<'a>(&self, value: &'a mut T) -> egui::Slider<'a> {
        let slider = egui::Slider::new(value, self.range.clone()).logarithmic(self.log).suffix(self.suffix);
        let slider = match self.step {
            Some(step) => slider.step_by(step),
            None => slider,
        };
        match self.decimals {
            Some(places) => slider.fixed_decimals(places),
            None => slider,
        }
    }

    fn text(&self, value: T) -> String {
        let v = value.to_f64();
        match self.decimals {
            Some(places) => format!("{v:.places$}{}", self.suffix),
            None if T::INTEGRAL => format!("{}{}", v.round(), self.suffix),
            None => format!("{v:.2}{}", self.suffix),
        }
    }

    /// Where `value` sits along the rail, from 0 to 1.
    fn share(&self, value: T) -> f64 {
        let (lo, hi, v) = (self.range.start().to_f64(), self.range.end().to_f64(), value.to_f64());
        let t = if self.log && lo > 0.0 { (v / lo).ln() / (hi / lo).ln() } else { (v - lo) / (hi - lo) };
        if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The value at share `t` of the rail, on the step when there is one.
    fn at(&self, t: f64) -> T {
        let (lo, hi) = (self.range.start().to_f64(), self.range.end().to_f64());
        let t = t.clamp(0.0, 1.0);
        let mut v = if self.log && lo > 0.0 { lo * (hi / lo).powf(t) } else { (hi - lo).mul_add(t, lo) };
        if let Some(step) = self.step {
            v = (v / step).round() * step;
        }
        if T::INTEGRAL {
            v = v.round();
        }
        T::from_f64(v.clamp(lo, hi))
    }
}

/// Points a finger may wander up or down on a thumb before the drag counts as a swipe.
const SWIPE_SLOP: f32 = 8.0;

/// A slider for a finger. A tap on the rail jumps there and only a drag that starts on the
/// thumb moves it, so a swipe up the list scrolls it instead of catching every rail it crosses.
fn touch_slider<T: egui::emath::Numeric>(ui: &mut Ui, value: &mut T, spec: &Spec<T>) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    let radius = 0.5 * ui.spacing().slider_rail_height;
    let rail = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width() - height, 2.0 * radius));
    let x_of = |t: f64| egui::lerp(rail.x_range(), t as f32);
    let t_of = |x: f32| f64::from(((x - rail.left()) / rail.width()).clamp(0.0, 1.0));
    let thumb_at = egui::pos2(x_of(spec.share(*value)), rect.center().y);
    let grip = ui.interact(
        egui::Rect::from_center_size(thumb_at, egui::vec2(height * 1.4, height)),
        response.id.with("thumb"),
        egui::Sense::drag(),
    );
    // A drag moves the thumb by the finger's travel across from where it started, never to
    // the finger, and once it has gone further up or down than across it hands the value back.
    let held = grip.id.with("from");
    if grip.drag_started() {
        let from = (value.to_f64(), grip.interact_pointer_pos().unwrap_or(thumb_at));
        ui.data_mut(|d| d.insert_temp(held, from));
    }
    let target = if grip.dragged() {
        let from = ui.data(|d| d.get_temp::<(f64, egui::Pos2)>(held));
        from.zip(grip.interact_pointer_pos()).map(|((original, start), pos)| {
            let travel = pos - start;
            let across = travel.y.abs() <= travel.x.abs().max(SWIPE_SLOP) && travel.x.abs() >= 1.0;
            if across {
                spec.at(spec.share(T::from_f64(original)) + f64::from(travel.x / rail.width()))
            } else {
                T::from_f64(original)
            }
        })
    } else if response.clicked() {
        response.interact_pointer_pos().map(|pos| spec.at(t_of(pos.x)))
    } else {
        None
    };
    if let Some(new) = target
        && new.to_f64().total_cmp(&value.to_f64()).is_ne()
    {
        *value = new;
        response.mark_changed();
    }
    let shown = spec.share(*value);
    let enabled = ui.is_enabled();
    let text = spec.text(*value);
    response.widget_info(|| egui::WidgetInfo::slider(enabled, value.to_f64(), &text));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rail, radius, PALETTE.track);
        let filled = egui::Rect::from_min_max(rail.min, egui::pos2(x_of(shown), rail.max.y));
        painter.rect_filled(filled, radius, PALETTE.accent);
        let centre = egui::pos2(x_of(shown), rect.center().y);
        let held = grip.dragged() || grip.hovered();
        let edge = if held { PALETTE.accent } else { PALETTE.ink };
        painter.circle(centre, 0.42 * height, PALETTE.track, Stroke::new(2.0, edge));
    }
    response | grip
}

/// A named slider: in the grid, the name then the slider and its value box; on a small screen,
/// the name and value on one line over a rail the full width, with `extra` beside the value.
fn slide<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    help: &str,
    value: &mut T,
    spec: &Spec<T>,
    extra: impl FnOnce(&mut Ui, &mut T),
) -> egui::Response {
    if compact(ui.ctx()) {
        let text = spec.text(*value);
        let id = ui
            .horizontal(|ui| {
                let id = name(ui, label, help);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(text).color(PALETTE.muted));
                    extra(ui, value);
                });
                id
            })
            .inner;
        return touch_slider(ui, value, spec).labelled_by(id);
    }
    let id = name(ui, label, help);
    let response = ui.add(spec.widget(value)).labelled_by(id);
    extra(ui, value);
    response
}

fn float(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, help: &str) -> egui::Response {
    let places = decimals(&range);
    slide(ui, label, help, value, &Spec { decimals: Some(places), ..Spec::new(range) }, |_, _| {})
}

fn whole(ui: &mut Ui, label: &str, value: &mut u32, range: RangeInclusive<u32>, help: &str) {
    slide(ui, label, help, value, &Spec::new(range), |_, _| {});
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
    spec: &Spec<T>,
) -> Option<egui::Response> {
    let mut on = value.is_some();
    if flag(ui, label, &mut on, help).changed() {
        *value = on.then_some(start);
    }
    let v = value.as_mut()?;
    let response = slide(ui, value_label, help, v, spec, |_, _| {});
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
        &Spec { suffix: " fps", ..Spec::new(limits::FPS_CAP) },
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
            add(ui);
        });
        return;
    }
    egui::Grid::new(id).num_columns(3).spacing([16.0, 10.0]).show(ui, add);
}

fn param(ui: &mut Ui, spec: &ParamSpec, settings: &mut Settings) {
    let value = settings.params.get_mut(spec.slot);
    let range = spec.min..=spec.max;
    let places = decimals(&range);
    let reset = |ui: &mut Ui, value: &mut f32| {
        if *value != 0.0 && ui.small_button("reset").on_hover_text("Back to the designed value").clicked() {
            *value = 0.0;
        }
    };
    slide(ui, spec.label, spec.help, value, &Spec { decimals: Some(places), ..Spec::new(range) }, reset);
    ui.end_row();
}

fn params(ui: &mut Ui, group: &mut ParamGroup, settings: &mut Settings) {
    ui.horizontal_wrapped(|ui| tabs(ui, group, ParamGroup::VARIANTS));
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
            &Spec { log: true, step: Some(0.1), decimals: Some(1), suffix: " MP", ..Spec::new(range) },
        );
        if response.is_some_and(|r| r.changed()) || mp.is_some() != r.max_pixels.is_some() {
            // Whole tenths of a megapixel, which at the top would round past the cap's bound.
            r.max_pixels = mp.map(|mp| {
                ((mp * 10.0).round() / 10.0 * MEGA).clamp(*limits::MAX_PIXELS.start(), *limits::MAX_PIXELS.end())
            });
        }
        let scale = Spec { step: Some(0.05), decimals: Some(2), ..Spec::new(limits::RENDER_SCALE) };
        slide(ui, "Render scale", "Per-axis scale of the shaded picture.", &mut r.render_scale, &scale, |_, _| {});
        ui.end_row();
        flag(ui, "Adapt to frame rate", &mut r.adaptive, "Shade fewer pixels while frames run long.");
    });
    ui.separator();
    ui.label(match info.passes {
        Some(p) => format!("{:.0} fps  |  field {:.1} ms  |  shade {:.1} ms", info.fps, p.field, p.shade),
        None => format!("{:.0} fps  |  pass times unavailable on this device", info.fps),
    });
    if let Some(s) = info.shading {
        ui.label(shading_text(&s));
        if s.lowered {
            let note = "Lowered while frames run long; it climbs back once they have room.";
            ui.label(RichText::new(note).small().color(PALETTE.accent));
        }
    }
}

/// What frames are shaded at, as the Render tab shows it.
fn shading_text(s: &Shading) -> String {
    let [w, h] = s.size;
    let [ww, wh] = s.window;
    let mut parts = vec![if s.size == s.window {
        format!("Shading every pixel, {w} x {h}")
    } else {
        format!("Shading {w} x {h} of {ww} x {wh} ({:.0}%)", 100.0 * f64::from(w) / f64::from(ww.max(1)))
    }];
    if s.field_step > 1 {
        parts.push(format!("field step {}", s.field_step));
    }
    if s.supersampling > 1 {
        parts.push(format!("{}x supersampled", s.supersampling));
    }
    parts.join(", ")
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
        flag(
            ui,
            "Pause when hidden",
            &mut w.pause_when_hidden,
            "Stop drawing while the window is covered or minimised.",
        );
        flag(
            ui,
            "Pause under fullscreen",
            &mut w.pause_under_fullscreen,
            "Stop drawing while another program fills the monitor.",
        );
        flag(ui, "Pause on battery", &mut w.pause_on_battery, "Stop drawing while the computer runs on battery.");
        if !w.pause_on_battery {
            limit(
                ui,
                ("Limit on battery", "Hold to a lower frame rate while on battery.", "Battery frame rate"),
                &mut w.battery_fps_cap,
                DEFAULT_FPS,
                &Spec { suffix: " fps", ..Spec::new(limits::FPS_CAP) },
            );
        }
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
        if info.can_share && ui.button("Copy link").on_hover_text("A link that opens with these settings").clicked() {
            commands.push(Command::CopyLink);
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
        let frame = egui::Frame::window(&ctx.global_style()).inner_margin(12);
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
                s.item_spacing.x = 8.0;
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if close_button(ui) {
                        commands.push(Command::Close);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Min).with_main_wrap(true), |ui| {
                        tabs(ui, &mut self.tab, Tab::VARIANTS);
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

    fn tap(harness: &mut Harness<'static, State>, at: egui::Pos2) {
        use egui::{Event, Modifiers, PointerButton};
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.event(Event::PointerMoved(at));
        harness.event(press(true));
        harness.event(press(false));
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

    #[rstest]
    fn copy_link_shows_only_where_the_page_can_share(#[values(false, true)] can_share: bool) {
        let state = State {
            menu: Menu { open: true, tab: Tab::Wallpaper, ..Menu::default() },
            settings: Settings::default(),
            commands: vec![],
        };
        let mut harness = Harness::builder().with_size([760.0, 760.0]).build_ui_state(
            move |ui, state: &mut State| {
                super::super::theme::install(ui.ctx());
                let info = Info { can_share, ..Info::default() };
                let commands = state.menu.show(ui, &mut state.settings, &info);
                state.commands.extend(commands);
            },
            state,
        );
        harness.run();
        let button = harness.query_by_label("Copy link");
        check!(button.is_some() == can_share);
        if let Some(button) = button {
            button.click();
            harness.run();
            check!(harness.state().commands.contains(&Command::CopyLink));
        }
    }

    const FULL: Shading =
        Shading { size: [1920, 1080], window: [1920, 1080], field_step: 1, supersampling: 1, lowered: false };

    #[rstest]
    #[case::every_pixel(FULL, "Shading every pixel, 1920 x 1080")]
    #[case::capped(Shading { size: [960, 540], ..FULL }, "Shading 960 x 540 of 1920 x 1080 (50%)")]
    #[case::coarse_field(Shading { size: [960, 540], field_step: 2, ..FULL }, "Shading 960 x 540 of 1920 x 1080 (50%), field step 2")]
    #[case::supersampled(Shading { supersampling: 2, ..FULL }, "Shading every pixel, 1920 x 1080, 2x supersampled")]
    fn the_render_tab_says_what_frames_are_shaded_at(#[case] shading: Shading, #[case] text: &str) {
        check!(shading_text(&shading) == text);
    }

    fn render_tab(info: Info) -> Harness<'static, State> {
        let state = State {
            menu: Menu { open: true, tab: Tab::Render, ..Menu::default() },
            settings: Settings::default(),
            commands: vec![],
        };
        let mut harness = Harness::builder().with_size([760.0, 900.0]).build_ui_state(
            move |ui, state: &mut State| {
                super::super::theme::install(ui.ctx());
                let commands = state.menu.show(ui, &mut state.settings, &info);
                state.commands.extend(commands);
            },
            state,
        );
        harness.run();
        harness
    }

    #[test]
    fn pass_times_read_as_unavailable_rather_than_zero_where_the_device_has_none() {
        let untimed = render_tab(Info { fps: 30.0, ..Info::default() });
        check!(untimed.query_by_label_contains("pass times unavailable").is_some());
        check!(untimed.query_by_label_contains("0.0 ms").is_none());
        let timed =
            render_tab(Info { fps: 30.0, passes: Some(PassTimes { field: 1.25, shade: 3.5 }), ..Info::default() });
        check!(timed.query_by_label_contains("field 1.2 ms").is_some());
        check!(timed.query_by_label_contains("unavailable").is_none());
    }

    #[rstest]
    fn the_render_tab_says_when_frames_running_long_lowered_the_shading(#[values(false, true)] lowered: bool) {
        let shading = Shading { size: [960, 540], field_step: 2, lowered, ..FULL };
        let harness = render_tab(Info { shading: Some(shading), ..Info::default() });
        check!(harness.query_by_label_contains("Shading 960 x 540").is_some());
        check!(harness.query_by_label_contains("Lowered while frames run long").is_some() == lowered);
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

    #[test]
    fn on_a_phone_a_swipe_up_the_menu_leaves_every_slider_alone() {
        let mut harness = sized(Tab::Params, Settings::default(), [390.0, 844.0]);
        harness.run();
        let shown: Vec<_> = CATALOG.iter().filter(|s| s.group == Group::Obstacle).collect();
        let top = harness.get_by_role_and_label(Role::Slider, shown[0].label).rect();
        for spec in &shown {
            let rail = harness.get_by_role_and_label(Role::Slider, spec.label).rect();
            let share = (0.0 - spec.min) / (spec.max - spec.min);
            let thumb =
                egui::pos2(rail.left() + rail.height() / 2.0 + (rail.width() - rail.height()) * share, rail.center().y);
            // From the thumb and from the bare rail, straight up past the first row.
            for from in [thumb, egui::pos2(rail.left() + 12.0, rail.center().y)] {
                drag(&mut harness, from, egui::vec2(1.5, top.top() - from.y - 40.0));
                check!(
                    harness.state().settings == Settings::default(),
                    "a swipe from {from:?} on {} changed it",
                    spec.label
                );
            }
        }
    }

    #[test]
    fn on_a_phone_tapping_a_rail_sets_the_value_there_and_dragging_its_thumb_moves_it() {
        let mut harness = sized(Tab::Params, Settings::default(), [390.0, 844.0]);
        harness.run();
        let spec = CATALOG.iter().find(|s| s.group == Group::Obstacle).expect("an obstacle parameter");
        let value = |h: &Harness<'static, State>| h.state().settings.params.get(spec.slot);
        let rail = harness.get_by_role_and_label(Role::Slider, spec.label).rect();
        tap(&mut harness, egui::pos2(rail.left() + 0.05 * rail.width(), rail.center().y));
        let tapped = value(&harness);
        check!(tapped < 0.0, "a tap near the left end lowered the value: {tapped}");
        let share = (tapped - spec.min) / (spec.max - spec.min);
        let thumb_x = rail.left() + rail.height() / 2.0 + (rail.width() - rail.height()) * share;
        drag(&mut harness, egui::pos2(thumb_x, rail.center().y), egui::vec2(rail.right() - thumb_x, 0.0));
        check!(value(&harness) > 0.0, "dragging the thumb to the right end raised it: {}", value(&harness));
        check!(harness.state().settings.clone().clamp().is_empty());
    }
}
