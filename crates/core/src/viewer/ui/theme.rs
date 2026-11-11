//! The menu's look, set once on the context the viewer and the tests both use. The menu sits
//! over a busy, high-contrast picture, so the panel is close to opaque and the type is large.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals};

/// Raw colours, named once.
mod swatch {
    use egui::Color32;

    pub(super) const INK: Color32 = Color32::from_rgb(0xf4, 0xf1, 0xeb);
    pub(super) const MUTED: Color32 = Color32::from_rgb(0xb9, 0xb5, 0xad);
    pub(super) const PANEL: Color32 = Color32::from_rgb(0x15, 0x16, 0x19);
    pub(super) const RAISED: Color32 = Color32::from_rgb(0x2b, 0x2d, 0x33);
    pub(super) const HOVER: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x45);
    pub(super) const EDGE: Color32 = Color32::from_rgb(0x3c, 0x3f, 0x47);
    pub(super) const AMBER: Color32 = Color32::from_rgb(0xf0, 0xa9, 0x3b);
    pub(super) const RED: Color32 = Color32::from_rgb(0xf0, 0x74, 0x66);
}

/// Colours by what they mean.
pub(crate) struct Palette {
    pub(crate) accent: Color32,
    pub(crate) warn: Color32,
    pub(crate) ink: Color32,
    pub(crate) muted: Color32,
    /// The track of a control that is off.
    pub(crate) track: Color32,
}

pub(crate) const PALETTE: Palette =
    Palette { accent: swatch::AMBER, warn: swatch::RED, ink: swatch::INK, muted: swatch::MUTED, track: swatch::RAISED };

/// Sizes shared across the menu, the hint and the hold ring.
pub(crate) struct Metrics {
    /// The menu's width on every tab, so switching tabs never moves its edges.
    pub(crate) menu_width: f32,
    pub(crate) tab_gap: f32,
    pub(crate) ring_radius: f32,
    pub(crate) ring_width: f32,
    pub(crate) toast_margin: f32,
}

pub(crate) const METRICS: Metrics =
    Metrics { menu_width: 560.0, tab_gap: 18.0, ring_radius: 26.0, ring_width: 4.0, toast_margin: 32.0 };

fn widgets(visuals: &mut Visuals) {
    let w = &mut visuals.widgets;
    for (state, fill) in [
        (&mut w.inactive, swatch::RAISED),
        (&mut w.hovered, swatch::HOVER),
        (&mut w.active, swatch::HOVER),
        (&mut w.open, swatch::HOVER),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.fg_stroke = Stroke::new(1.5, swatch::INK);
        state.corner_radius = CornerRadius::same(6);
    }
    w.noninteractive.fg_stroke = Stroke::new(1.0, swatch::INK);
    w.noninteractive.bg_stroke = Stroke::new(1.0, swatch::EDGE);
    w.hovered.bg_stroke = Stroke::new(1.0, swatch::AMBER.linear_multiply(0.6));
    w.active.bg_stroke = Stroke::new(1.5, swatch::AMBER);
}

pub(crate) fn install(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.window_fill = swatch::PANEL;
    visuals.panel_fill = swatch::PANEL;
    visuals.extreme_bg_color = swatch::RAISED;
    visuals.window_stroke = Stroke::new(1.0, swatch::EDGE);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.window_shadow = Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(160) };
    visuals.selection.bg_fill = swatch::AMBER;
    visuals.selection.stroke = Stroke::new(1.5, swatch::PANEL);
    visuals.slider_trailing_fill = true;
    visuals.hyperlink_color = swatch::AMBER;
    visuals.override_text_color = Some(swatch::INK);
    visuals.weak_text_color = Some(swatch::MUTED);
    widgets(&mut visuals);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(15.0, FontFamily::Monospace)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(10.0, 10.0);
        s.button_padding = egui::vec2(10.0, 5.0);
        s.window_margin = Margin::same(16);
        s.slider_width = 210.0;
        s.slider_rail_height = 8.0;
        // Tall enough for a thumb that is easy to grab, and wide enough that a column of value
        // boxes beside sliders lines up.
        s.interact_size = egui::vec2(64.0, 26.0);
        style.interaction.tooltip_delay = 0.3;
    });
}
