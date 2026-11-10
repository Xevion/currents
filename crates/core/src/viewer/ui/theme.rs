//! The menu's look, set once on the context the viewer and the tests both use.

use egui::{Color32, CornerRadius, Margin, Stroke, Visuals};

/// Raw colours, named once.
mod swatch {
    use egui::Color32;

    pub(super) const INK: Color32 = Color32::from_rgb(0xe8, 0xe4, 0xdc);
    pub(super) const PANEL: Color32 = Color32::from_rgba_premultiplied(0x16, 0x17, 0x1a, 0xe6);
    pub(super) const EDGE: Color32 = Color32::from_rgb(0x34, 0x36, 0x3c);
    pub(super) const AMBER: Color32 = Color32::from_rgb(0xf0, 0xa9, 0x3b);
    pub(super) const RED: Color32 = Color32::from_rgb(0xe8, 0x6a, 0x5c);
}

/// Colours by what they mean.
pub(crate) struct Palette {
    pub(crate) accent: Color32,
    pub(crate) warn: Color32,
    pub(crate) ink: Color32,
}

pub(crate) const PALETTE: Palette = Palette { accent: swatch::AMBER, warn: swatch::RED, ink: swatch::INK };

/// Sizes shared across the menu, the hint and the hold ring.
pub(crate) struct Metrics {
    pub(crate) menu_width: f32,
    pub(crate) ring_radius: f32,
    pub(crate) ring_width: f32,
    pub(crate) toast_margin: f32,
}

pub(crate) const METRICS: Metrics =
    Metrics { menu_width: 380.0, ring_radius: 22.0, ring_width: 3.0, toast_margin: 24.0 };

pub(crate) fn install(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.window_fill = swatch::PANEL;
    visuals.panel_fill = swatch::PANEL;
    visuals.window_stroke = Stroke::new(1.0, swatch::EDGE);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.selection.bg_fill = swatch::AMBER.linear_multiply(0.45);
    visuals.hyperlink_color = swatch::AMBER;
    visuals.override_text_color = Some(swatch::INK);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.window_margin = Margin::same(12);
        style.spacing.slider_width = 170.0;
        style.interaction.tooltip_delay = 0.3;
    });
}
