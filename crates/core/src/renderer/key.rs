//! Keys: the traced liquid over the view at a fixed time, which the frames after it read
//! instead of tracing every pixel's whole history again. A key is traced when none came
//! before it, and otherwise carried forward from the key it replaces.

use std::ops::Range;

pub(super) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;

// The cover's stripe-plane homography, focal length and flow speed, as the shaders have them.
const HA: f32 = 106.348;
const HB: f32 = -65.956;
const HC: f32 = 21.957;
const HG: f32 = -0.02587;
const HH: f32 = 1.10028;
const FOCAL: f32 = 1.2;
const FLOW_SPEED: f32 = 4.0;

/// Seconds of flow a key serves before the next replaces it. A frame steps back at most this
/// far, so shorter is cheaper per frame and dearer in keys.
const SPAN: f32 = 0.5;
/// Most of the view's width and height a key adds on each side for liquid arriving from off screen.
const MAX_MARGIN: f32 = 0.125;
/// Below this span the liquid crosses the view too fast for a key to pay for itself.
const MIN_SPAN: f32 = 0.02;
/// Share of a key's span over which its successor is rendered, a few stripes each frame, so
/// no one frame pays for a whole key.
const PREPARE: f32 = 0.6;
/// Vertical stripes a key is rendered in. Some parts of the picture cost far more to trace
/// than others, so the stripes are taken in an order that spreads each frame's across it.
const STRIPES: u32 = 64;
/// Fastest liquid allowed for when sizing the margin, along and across the flow, as multiples
/// of the flow speed. Faster liquid than this is traced in full by the frames that meet it.
const PEAK_SPEED: [f32; 2] = [1.5, 0.5];
/// Frames a key refitted round a moving view is rendered over.
const REFIT_FRAMES: u32 = 6;
/// Frames a key traced afresh for a view that has come to rest is rendered over.
const RETRACE_FRAMES: u32 = 24;
/// Frames of the view's movement a key must still hold before it is refitted, and the frames
/// of movement the refitted key allows for.
const CHECK_LEAD: f32 = 8.0;
const PLAN_LEAD: f32 = 20.0;
/// Furthest a key reaches after a moving view, as a share of the view's size.
const MAX_LEAD: f32 = 0.25;
/// A key with less than this share of the detail the view shows is traced afresh.
const SOFT: f32 = 0.85;

type Vec3 = [f32; 3];

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scaled(a: Vec3, k: f32) -> Vec3 {
    a.map(|v| v * k)
}

fn normalized(a: Vec3) -> Vec3 {
    scaled(a, dot(a, a).sqrt().recip())
}

/// The stripe plane as the camera sees it, the same construction as the shaders' `camera`.
struct Plane {
    normal: Vec3,
    distance: f32,
    downstream: Vec3,
    across: Vec3,
}

impl Plane {
    fn new() -> Self {
        let np = [HG, HH, (1.0 + f32::midpoint(HG, HH)) / FOCAL];
        let a = [HA, HB, (HC + f32::midpoint(HA, HB)) / FOCAL];
        let normal = normalized(np);
        let k = dot(a, normal);
        let ap = [a[0] - k * normal[0], a[1] - k * normal[1], a[2] - k * normal[2]];
        let across = normalized(ap);
        let cross = [
            across[1] * normal[2] - across[2] * normal[1],
            across[2] * normal[0] - across[0] * normal[2],
            across[0] * normal[1] - across[1] * normal[0],
        ];
        let downstream = if cross[0] < 0.0 { scaled(cross, -1.0) } else { cross };
        Self { normal, distance: dot(ap, ap).sqrt() / dot(np, np).sqrt(), downstream, across }
    }

    /// Plane position under a point of the cover, or `None` at and past the horizon.
    fn under(&self, cover: [f32; 2]) -> Option<[f32; 2]> {
        let ray = [cover[0] - 0.5, cover[1] - 0.5, FOCAL];
        let facing = dot(self.normal, ray);
        (facing > 1e-6).then(|| {
            let hit = scaled(ray, self.distance / facing);
            [dot(hit, self.downstream), dot(hit, self.across)]
        })
    }

    /// Point of the cover over a plane position.
    fn over(&self, q: [f32; 2]) -> [f32; 2] {
        let p: Vec3 =
            std::array::from_fn(|i| self.downstream[i] * q[0] + self.across[i] * q[1] + self.normal[i] * self.distance);
        [p[0] / p[2] * FOCAL + 0.5, p[1] / p[2] * FOCAL + 0.5]
    }
}

/// How far liquid on screen was from where it is now, `SPAN` earlier at the peak speed, as the
/// largest fraction of the view's width or height over a grid of points; `None` when the view
/// reaches the horizon.
fn reach(view: [f32; 4]) -> Option<f32> {
    const GRID: u8 = 5;
    let plane = Plane::new();
    let size = [view[2] - view[0], view[3] - view[1]];
    let mut reach = 0.0f32;
    for j in 0..GRID {
        for i in 0..GRID {
            let at = |n: u8, axis: usize| view[axis] + size[axis] * f32::from(n) / f32::from(GRID - 1);
            let cover = [at(i, 0), at(j, 1)];
            let q = plane.under(cover)?;
            for side in [-1.0, 1.0] {
                let travel = [PEAK_SPEED[0] * FLOW_SPEED * SPAN, side * PEAK_SPEED[1] * FLOW_SPEED * SPAN];
                let before = plane.over([q[0] - travel[0], q[1] - travel[1]]);
                for axis in 0..2 {
                    reach = reach.max((before[axis] - cover[axis]).abs() / size[axis].abs());
                }
            }
        }
    }
    reach.is_finite().then_some(reach)
}

/// The span a key over `view` serves and the margin it adds on each side, as fractions of the
/// view's width and height; `None` where a key would not pay: the view reaches the horizon, or
/// the liquid crosses it within a few frames.
fn terms(view: [f32; 4], field: [u32; 2]) -> Option<(f32, [f32; 2])> {
    let reach = reach(view)?;
    let span = if reach > MAX_MARGIN { SPAN * MAX_MARGIN / reach } else { SPAN };
    // three texels more for the interpolation's footprint
    (span >= MIN_SPAN).then(|| (span, std::array::from_fn(|axis| reach.min(MAX_MARGIN) + 3.0 / field[axis] as f32)))
}

fn extent(view: [f32; 4]) -> [f32; 2] {
    [view[2] - view[0], view[3] - view[1]]
}

/// The smallest rectangle holding both `a` and `b`.
fn joined(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn grown(view: [f32; 4], by: [f32; 2]) -> [f32; 4] {
    [view[0] - by[0], view[1] - by[1], view[2] + by[0], view[3] + by[1]]
}

/// Where `view` gets to after `frames` more frames each changing it by `drift`, no edge moving
/// further than a share of the view's size.
fn ahead(view: [f32; 4], drift: [f32; 4], frames: f32) -> [f32; 4] {
    let size = extent(view);
    std::array::from_fn(|i| {
        let most = MAX_LEAD * size[i % 2];
        view[i] + (drift[i] * frames).clamp(-most, most)
    })
}

/// When a key is taken, what it spans and how long it serves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Key {
    pub(super) time: f32,
    /// Seconds after `time` the key serves.
    pub(super) span: f32,
    /// Part of the cover it spans: the view, a margin, and where the view was heading.
    pub(super) view: [f32; 4],
    /// Texels across it, as fine as the field was.
    pub(super) size: [u32; 2],
    /// Texels per unit of the cover's width its liquid was resolved at: its own, or fewer when
    /// carried forward from a coarser key.
    pub(super) detail: f32,
}

impl Key {
    /// The key for a frame at `time` showing `view` with a field of `field` texels, or `None`
    /// where one would not pay.
    ///
    /// Key times sit on a grid of the span, so a frame's key depends only on the frame.
    pub(super) fn plan(time: f32, view: [f32; 4], field: [u32; 2]) -> Option<Self> {
        Self::around(time, view, view, field)
    }

    /// As `plan`, for a view on its way to `heading`: the key spans both.
    fn around(time: f32, view: [f32; 4], heading: [f32; 4], field: [u32; 2]) -> Option<Self> {
        let (span, margin) = terms(view, field)?;
        let size = extent(view);
        let by = [margin[0] * size[0], margin[1] * size[1]];
        // a heading that takes the key to the horizon is left out: nothing can be traced there
        let both = Some(grown(joined(view, heading), by)).filter(|&all| reach(all).is_some());
        let bounds = both.unwrap_or_else(|| grown(view, by));
        let across = extent(bounds);
        Some(Self {
            time: (time / span).floor() * span,
            span,
            view: bounds,
            size: std::array::from_fn(|axis| (field[axis] as f32 * across[axis] / size[axis]).ceil() as u32),
            detail: field[0] as f32 / size[0],
        })
    }

    /// The columns of the `stripe`-th stripe in rendering order: successive stripes land far
    /// apart, each halving a gap the earlier ones left.
    pub(super) fn stripe_columns(&self, stripe: u32) -> Range<u32> {
        let place = stripe.reverse_bits() >> (u32::BITS - STRIPES.trailing_zeros());
        place * self.size[0] / STRIPES..(place + 1) * self.size[0] / STRIPES
    }

    /// Whether a frame at `time` falls in the stretch this key serves.
    #[cfg(test)]
    fn serves(&self, time: f32) -> bool {
        time >= self.time && time < self.time + self.span
    }

    /// Whether a frame at `time` can still read this key, at the cost of a longer step back.
    fn reaches(&self, time: f32) -> bool {
        time >= self.time && time < 2.0f32.mul_add(self.span, self.time)
    }

    /// Whether `bounds` lies within the key, give or take a texel.
    fn holds(&self, bounds: [f32; 4]) -> bool {
        let slack = (self.view[2] - self.view[0]) / self.size[0] as f32;
        bounds[0] >= self.view[0] - slack
            && bounds[1] >= self.view[1] - slack
            && bounds[2] <= self.view[2] + slack
            && bounds[3] <= self.view[3] + slack
    }
}

/// Stripes of a key, in rendering order, to render into one of the two key textures.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Slice {
    pub(super) slot: usize,
    pub(super) key: Key,
    /// The key in the other texture to carry forward into this one; `None` traces it afresh.
    pub(super) from: Option<Key>,
    pub(super) stripes: Range<u32>,
}

impl Slice {
    /// Whether this is a large share of a key, enough to make its frame visibly late.
    pub(super) fn is_heavy(&self) -> bool {
        self.stripes.len() as u32 * 4 > STRIPES
    }
}

/// A key being rendered to take over from the one held.
#[derive(Clone, Copy)]
struct Coming {
    key: Key,
    /// Stripes rendered so far.
    done: u32,
    /// Whether it is carried forward from the key held, not traced afresh.
    carried: bool,
    /// Frames it is spread over when the picture is waiting on it; zero for a successor
    /// prepared ahead of its time.
    frames: u32,
}

/// The key frames read now and the one being rendered to follow it, each in its own texture.
///
/// Keys span part of the cover, not the screen, so a key outlives changes of view. One the
/// view is leaving is refitted: carried into a key around where the view is heading, in a few
/// frames. One the view has closed in on has too little detail, and is traced afresh once the
/// view rests.
#[derive(Default)]
pub(super) struct Keys {
    held: Option<Key>,
    slot: usize,
    coming: Option<Coming>,
    busy: bool,
}

impl Keys {
    /// Drops both keys: the next frame does not continue the last.
    pub(super) fn clear(&mut self) {
        *self = Self { slot: self.slot, ..Self::default() };
    }

    /// The key to read and the texture it is in.
    pub(super) fn held(&self) -> Option<(usize, Key)> {
        self.held.map(|key| (self.slot, key))
    }

    /// Whether drawing the same frame again would bring a better key closer.
    pub(super) const fn busy(&self) -> bool {
        self.busy
    }

    /// Moves on to a frame at `time`, `dt` after the last one, whose `view` changed by `drift`
    /// since then, and returns the stripes to render before it is drawn: a whole key when none
    /// can be read, otherwise the next few stripes of the one coming.
    pub(super) fn advance(
        &mut self,
        time: f32,
        dt: f32,
        view: [f32; 4],
        drift: [f32; 4],
        field: [u32; 2],
    ) -> Option<Slice> {
        self.busy = false;
        let Some(wanted) = Key::plan(time, view, field) else {
            self.clear();
            return None;
        };
        if let Some(coming) = self.coming.filter(|coming| coming.done == STRIPES && coming.key.time <= time) {
            self.slot ^= 1;
            self.held = Some(coming.key);
            self.coming = None;
        }
        let heading = ahead(view, drift, PLAN_LEAD);
        let Some(held) = self.held.filter(|key| key.reaches(time)) else {
            self.coming = None;
            self.held = Key::around(time, view, heading, field);
            return self.held.map(|key| Slice { slot: self.slot, key, from: None, stripes: 0..STRIPES });
        };

        let size = extent(view);
        let margin =
            [0.5 * (wanted.view[2] - wanted.view[0] - size[0]), 0.5 * (wanted.view[3] - wanted.view[1] - size[1])];
        // half the margin is enough to be going on with, which keeps a slow pan from refitting every frame
        let needed = grown(joined(view, ahead(view, drift, CHECK_LEAD)), margin.map(|m| 0.5 * m));
        let leaving = !held.holds(needed);
        let still = drift.iter().all(|&edge| edge == 0.0);
        let soft = held.detail < SOFT * wanted.detail;
        // a successor prepared ahead gives way to a key the picture is waiting on, and a
        // fresh trace is put off while the view is still changing
        if self.coming.is_some_and(|coming| {
            if coming.frames == 0 { leaving || (soft && still) } else { !coming.carried && !still }
        }) {
            self.coming = None;
        }
        if self.coming.is_none() {
            self.coming = if leaving {
                let key = Key::around(time, view, heading, field);
                key.map(|key| Coming {
                    key: Key { time: key.time.max(held.time), detail: key.detail.min(held.detail), ..key },
                    done: 0,
                    carried: true,
                    frames: REFIT_FRAMES,
                })
            } else if soft && still {
                Some(Coming { key: wanted, done: 0, carried: false, frames: RETRACE_FRAMES })
            } else {
                Key::around(1.5f32.mul_add(held.span, held.time), view, heading, field)
                    .filter(|key| key.time > held.time)
                    .map(|key| Coming {
                        key: Key { detail: key.detail.min(held.detail), ..key },
                        done: 0,
                        carried: true,
                        frames: 0,
                    })
            };
        }
        let coming = self.coming.as_mut()?;
        self.busy = coming.frames > 0 || leaving || soft;
        let left = STRIPES - coming.done;
        let span = held.span.min(coming.key.span);
        let paced = if dt > 0.0 { (STRIPES as f32 * dt / (PREPARE * span)).ceil() as u32 } else { 0 };
        let hurried = if coming.frames > 0 { STRIPES.div_ceil(coming.frames) } else { 0 };
        let stripes = paced.max(hurried).min(left);
        if stripes == 0 {
            return None;
        }
        let range = coming.done..coming.done + stripes;
        coming.done += stripes;
        Some(Slice { slot: self.slot ^ 1, key: coming.key, from: coming.carried.then_some(held), stripes: range })
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;
    use crate::shaders::Sources;

    const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const STILL: [f32; 4] = [0.0; 4];

    #[rstest]
    #[case("HA", HA)]
    #[case("HB", HB)]
    #[case("HC", HC)]
    #[case("HG", HG)]
    #[case("HH", HH)]
    #[case("FOCAL", FOCAL)]
    #[case("FLOW_SPEED", FLOW_SPEED)]
    fn constants_match_the_embedded_shaders(#[case] name: &str, #[case] value: f32) {
        let sources = Sources::embedded();
        let text = format!("{}\n{}", sources.common, sources.field);
        let declared = text
            .lines()
            .find_map(|line| line.strip_prefix(&format!("const {name} = ")))
            .and_then(|rest| rest.split(';').next())
            .and_then(|number| number.trim().parse::<f32>().ok());
        check!(declared == Some(value));
    }

    #[rstest]
    #[case([0.2, 0.3])]
    #[case([0.543, 0.661])]
    #[case([0.9, 0.95])]
    fn the_plane_maps_the_cover_there_and_back(#[case] cover: [f32; 2]) {
        let plane = Plane::new();
        assert!(let Some(q) = plane.under(cover));
        let back = plane.over(q);
        check!((back[0] - cover[0]).abs() < 1e-4 && (back[1] - cover[1]).abs() < 1e-4);
    }

    #[test]
    fn the_whole_cover_gets_a_full_span_and_a_modest_margin() {
        assert!(let Some(key) = Key::plan(7.3, FULL, [1920, 1920]));
        check!(key.span >= SPAN);
        check!(key.serves(7.3) && !key.serves(key.time + key.span));
        check!(key.view[0] < 0.0 && key.view[0] > -MAX_MARGIN - 0.01);
        check!(key.size[0] > 1920 && key.size[0] < 2420);
    }

    #[test]
    fn zooming_in_shortens_the_span_until_no_key_pays() {
        let around = |half: f32| [0.6 - half, 0.8 - half, 0.6 + half, 0.8 + half];
        assert!(let Some(near) = Key::plan(7.3, around(0.05), [1000, 1000]));
        check!(near.span < SPAN);
        check!(Key::plan(7.3, around(0.0005), [1000, 1000]) == None);
    }

    #[test]
    fn stripes_cover_every_column_once() {
        assert!(let Some(key) = Key::plan(7.3, FULL, [1000, 700]));
        let mut columns: Vec<u32> = (0..STRIPES).flat_map(|stripe| key.stripe_columns(stripe)).collect();
        columns.sort_unstable();
        check!(columns == (0..key.size[0]).collect::<Vec<_>>());
    }

    #[test]
    fn a_run_of_frames_renders_one_whole_key_then_its_successors_a_few_stripes_at_a_time() {
        const FIELD: [u32; 2] = [800, 800];
        let dt = 0.4 / 60.0;
        let mut keys = Keys::default();
        assert!(let Some(first) = keys.advance(7.0, dt, FULL, STILL, FIELD));
        check!(first.stripes == (0..STRIPES) && first.is_heavy() && first.from == None);
        assert!(let Some((slot, held)) = keys.held());
        check!((first.slot, first.key) == (slot, held));

        let mut done = 0;
        let mut frame = 1;
        while keys.held().is_some_and(|(_, key)| key == held) {
            let slice = keys.advance(dt.mul_add(frame as f32, 7.0), dt, FULL, STILL, FIELD);
            if let Some(slice) = slice.filter(|_| keys.held().is_some_and(|(_, key)| key == held)) {
                check!(slice.slot != slot && slice.stripes.start == done && !slice.is_heavy());
                check!(slice.from == Some(held));
                done = slice.stripes.end;
            }
            frame += 1;
        }
        assert!(let Some((next_slot, next)) = keys.held());
        check!(next_slot != slot && next.time > held.time && done == STRIPES);
    }

    #[test]
    fn a_frame_far_ahead_gets_a_whole_new_key() {
        let mut keys = Keys::default();
        keys.advance(7.0, 0.01, FULL, STILL, [800, 800]);
        keys.advance(7.01, 0.01, FULL, STILL, [800, 800]);
        assert!(let Some(slice) = keys.advance(20.0, 12.99, FULL, STILL, [800, 800]));
        check!(slice.stripes == (0..STRIPES) && slice.key.serves(20.0));
    }

    /// Runs `frames` frames from time 7, the view moving by `drift` each frame from `view`, and
    /// returns the slices rendered and the view the run ends on.
    fn run(keys: &mut Keys, view: [f32; 4], drift: [f32; 4], dt: f32, frames: u32) -> (Vec<Slice>, [f32; 4]) {
        const FIELD: [u32; 2] = [800, 800];
        let mut at = view;
        let slices = (0..frames)
            .filter_map(|frame| {
                let moved = if frame == 0 { STILL } else { drift };
                at = std::array::from_fn(|i| at[i] + moved[i]);
                keys.advance(dt.mul_add(frame as f32, 7.0), dt, at, moved, FIELD)
            })
            .collect();
        (slices, at)
    }

    #[rstest]
    #[case::running(0.4 / 60.0)]
    #[case::paused(0.0)]
    fn a_panning_view_keeps_its_key_and_refits_it_ahead_of_the_view(#[case] dt: f32) {
        let start = [0.3, 0.3, 0.6, 0.6];
        let pan = [0.004, 0.0, 0.004, 0.0];
        let mut keys = Keys::default();
        let (slices, view) = run(&mut keys, start, pan, dt, 60);
        check!(slices[0].from == None && slices[0].is_heavy());
        // every key after the first is carried from the one before, a few stripes a frame
        check!(slices[1..].iter().all(|slice| slice.from.is_some() && !slice.is_heavy()));
        assert!(let Some((_, held)) = keys.held());
        check!(held.holds(view) && held.view[2] > slices[0].key.view[2]);
    }

    #[test]
    fn a_view_that_closed_in_is_traced_afresh_once_it_rests() {
        let mut keys = Keys::default();
        let (_, view) = run(&mut keys, [0.2, 0.2, 0.8, 0.8], [0.004, 0.004, -0.004, -0.004], 0.4 / 60.0, 30);
        assert!(let Some((_, soft)) = keys.held());
        check!(keys.busy());
        let (slices, _) = run(&mut keys, view, STILL, 0.0, 40);
        assert!(let Some((_, held)) = keys.held());
        check!(held.detail > 1.2 * soft.detail && !keys.busy());
        check!(slices.iter().all(|slice| !slice.is_heavy()));
        check!(slices.iter().any(|slice| slice.from.is_none()));
    }

    #[test]
    fn a_view_past_the_horizon_gets_no_key() {
        check!(Key::plan(0.0, [0.0, -3.0, 1.0, 1.0], [800, 800]) == None);
    }
}
