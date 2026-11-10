//! Short messages near the bottom of the window that fade on their own.

use egui::{Align2, Id, Order, RichText};
use web_time::{Duration, Instant};

use super::theme::{METRICS, PALETTE};

const LIFE: Duration = Duration::from_millis(2500);
const FADE: Duration = Duration::from_millis(600);
/// Toasts beyond this many push the oldest out.
const MOST: usize = 4;

struct Toast {
    text: String,
    warn: bool,
    born: Instant,
}

#[derive(Default)]
pub(crate) struct Toasts {
    live: Vec<Toast>,
}

impl Toasts {
    /// Shows `text`; the same text already showing starts its life again instead of stacking.
    pub(crate) fn push(&mut self, text: String, warn: bool, now: Instant) {
        self.live.retain(|t| t.text != text);
        self.live.push(Toast { text, warn, born: now });
        if self.live.len() > MOST {
            self.live.remove(0);
        }
    }

    pub(crate) fn any(&mut self, now: Instant) -> bool {
        self.live.retain(|t| now - t.born < LIFE + FADE);
        !self.live.is_empty()
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, now: Instant) {
        if !self.any(now) {
            return;
        }
        let margin = METRICS.toast_margin;
        egui::Area::new(Id::unique("toasts"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -margin])
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.live {
                    let age = now - toast.born;
                    let fade = age.saturating_sub(LIFE).as_secs_f32() / FADE.as_secs_f32();
                    let alpha = (1.0 - fade).clamp(0.0, 1.0);
                    let ink = if toast.warn { PALETTE.warn } else { PALETTE.ink };
                    egui::Frame::popup(ui.style()).multiply_with_opacity(alpha).show(ui, |ui| {
                        ui.label(RichText::new(&toast.text).color(ink.gamma_multiply(alpha)));
                    });
                }
            });
        ctx.request_repaint();
    }
}
