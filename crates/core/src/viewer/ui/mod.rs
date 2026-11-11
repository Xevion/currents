//! The egui layer over the picture: the settings menu, the toasts and the hold ring. It runs
//! only while one of them is showing, so a closed menu costs the frame nothing.

pub(crate) mod panels;
mod theme;
mod toast;

use std::f32::consts::TAU;

use egui::epaint::ClippedPrimitive;
use egui::{Id, LayerId, Order, Stroke, TexturesDelta};
use web_time::Instant;
use winit::event::WindowEvent;
use winit::window::Window;

use self::panels::Menu;
pub(crate) use self::panels::{Command, Info, Shading};
#[cfg(test)]
pub(crate) use self::theme::install as install_theme;
use self::theme::{METRICS, PALETTE};
use self::toast::Toasts;
use crate::settings::Settings;

/// One frame of the overlay, ready for the GPU.
pub(crate) struct Painted {
    pub(crate) primitives: Vec<ClippedPrimitive>,
    pub(crate) textures: TexturesDelta,
    /// Target pixels per point.
    pub(crate) pixels_per_point: f32,
}

/// A hold on the ball in progress: where the pointer is, in window pixels, and how far along.
#[derive(Clone, Copy)]
pub(crate) struct Ring {
    pub(crate) at: [f64; 2],
    pub(crate) progress: f32,
}

pub(crate) struct Overlay {
    ctx: egui::Context,
    state: egui_winit::State,
    menu: Menu,
    toasts: Toasts,
}

impl Overlay {
    pub(crate) fn new(window: &Window, max_texture_side: u32) -> Self {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_texture_side as usize),
        );
        Self { ctx, state, menu: Menu::default(), toasts: Toasts::default() }
    }

    pub(crate) const fn menu_open(&self) -> bool {
        self.menu.open
    }

    /// Opening the menu clears hints about how to open it.
    pub(crate) fn set_menu_open(&mut self, open: bool) {
        if open {
            self.toasts.dismiss_hints();
        }
        self.menu.open = open;
    }

    pub(crate) fn toast(&mut self, text: String, warn: bool) {
        self.toasts.push(text, warn, Instant::now());
    }

    /// Whether anything is showing, which is when frames run egui at all.
    pub(crate) fn active(&mut self, now: Instant, ring: Option<Ring>) -> bool {
        self.menu.open || ring.is_some() || self.toasts.any(now)
    }

    /// Hands `event` to egui; true when egui took it for itself. Input that arrives while
    /// nothing shows is left out, since no frame would ever collect it.
    pub(crate) fn on_event(&mut self, window: &Window, event: &WindowEvent, active: bool) -> bool {
        let bookkeeping = matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ModifiersChanged(_)
        );
        if !active && !bookkeeping {
            return false;
        }
        self.state.on_window_event(window, event).consumed
    }

    /// Whether the pointer is over or held by an egui window.
    pub(crate) fn owns_pointer(&self) -> bool {
        self.menu.open && (self.ctx.egui_is_using_pointer() || self.ctx.is_pointer_over_egui())
    }

    /// Whether the open menu covers `at`, in window pixels. A touch has no hover before it
    /// lands, so this asks where the menu is drawn rather than where the pointer last was.
    pub(crate) fn covers(&self, at: [f64; 2]) -> bool {
        let ppp = f64::from(self.ctx.pixels_per_point());
        let pos = egui::pos2((at[0] / ppp) as f32, (at[1] / ppp) as f32);
        self.menu.open && self.ctx.layer_id_at(pos).is_some_and(|layer| layer.order != Order::Background)
    }

    /// Runs one frame of the overlay, painted for a target the window's size.
    pub(crate) fn frame(
        &mut self,
        window: &Window,
        settings: &mut Settings,
        info: &Info,
        ring: Option<Ring>,
    ) -> (Painted, Vec<Command>) {
        let now = Instant::now();
        let raw = self.state.take_egui_input(window);
        let mut commands = Vec::new();
        let Self { ctx, menu, toasts, .. } = self;
        let output = ctx.run_ui(raw, |ui| {
            commands = menu.show(ui, settings, info);
            toasts.show(ui.ctx(), now);
            if let Some(ring) = ring {
                draw_ring(ui.ctx(), ring);
            }
        });
        self.state.handle_platform_output(window, output.platform_output);
        let pixels_per_point = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, pixels_per_point);
        (Painted { primitives, textures: output.textures_delta, pixels_per_point }, commands)
    }

    pub(crate) fn copy(&self, text: String) {
        self.ctx.copy_text(text);
    }
}

fn draw_ring(ctx: &egui::Context, ring: Ring) {
    let ppp = ctx.pixels_per_point();
    let centre = egui::pos2(ring.at[0] as f32 / ppp, ring.at[1] as f32 / ppp);
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("hold ring")));
    let radius = METRICS.ring_radius;
    painter.circle_stroke(centre, radius, Stroke::new(METRICS.ring_width, PALETTE.ink.gamma_multiply(0.25)));
    let steps = (48.0 * ring.progress).ceil().max(1.0) as usize;
    let arc: Vec<egui::Pos2> = (0..=steps)
        .map(|i| {
            let angle = TAU * ring.progress * i as f32 / steps as f32 - TAU / 4.0;
            centre + radius * egui::vec2(angle.cos(), angle.sin())
        })
        .collect();
    painter.add(egui::Shape::line(arc, Stroke::new(METRICS.ring_width, PALETTE.accent)));
    ctx.request_repaint();
}
