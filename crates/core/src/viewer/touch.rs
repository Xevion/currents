use winit::event::TouchPhase;

/// What the fingers on the screen asked the camera to do, in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Gesture {
    Pan([f64; 2]),
    /// Two fingers: their midpoint moved by `pan` and their spread scaled the view by `factor`.
    Pinch {
        centre: [f64; 2],
        pan: [f64; 2],
        factor: f32,
    },
}

/// A finger on the screen: where it is now and where it first touched.
#[derive(Clone, Copy)]
struct Point {
    id: u64,
    at: [f64; 2],
    start: [f64; 2],
}

/// Tracks up to two touches and turns their movement into pans and pinches; further fingers are ignored.
#[derive(Default)]
pub(crate) struct Touches {
    points: Vec<Point>,
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [f64::midpoint(a[0], b[0]), f64::midpoint(a[1], b[1])]
}

impl Touches {
    /// Whether any finger is down.
    pub(crate) fn any(&self) -> bool {
        !self.points.is_empty()
    }

    /// Where the only finger down is, while it has stayed within `slop` pixels of where it
    /// first touched: a press being held rather than a pan.
    pub(crate) fn still(&self, slop: f64) -> Option<[f64; 2]> {
        match self.points.as_slice() {
            [p] if distance(p.at, p.start) <= slop => Some(p.at),
            _ => None,
        }
    }

    /// Stops following the fingers down now, so their movement until they lift does nothing.
    pub(crate) fn forget(&mut self) {
        self.points.clear();
    }

    pub(crate) fn event(&mut self, phase: TouchPhase, id: u64, pos: [f64; 2]) -> Option<Gesture> {
        match phase {
            TouchPhase::Started => {
                if self.points.len() < 2 {
                    self.points.push(Point { id, at: pos, start: pos });
                }
                None
            },
            TouchPhase::Moved => {
                let i = self.points.iter().position(|p| p.id == id)?;
                let old = std::mem::replace(&mut self.points[i].at, pos);
                let Some(&Point { at: other, .. }) = self.points.get(1 - i) else {
                    return Some(Gesture::Pan([pos[0] - old[0], pos[1] - old[1]]));
                };
                let (before, after) = (midpoint(old, other), midpoint(pos, other));
                let spread = distance(pos, other);
                let factor = if spread > 1.0 { (distance(old, other) / spread) as f32 } else { 1.0 };
                Some(Gesture::Pinch { centre: after, pan: [after[0] - before[0], after[1] - before[1]], factor })
            },
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.points.retain(|p| p.id != id);
                None
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn one_finger_pans_by_its_movement() {
        let mut t = Touches::default();
        t.event(TouchPhase::Started, 1, [10.0, 10.0]);
        check!(t.event(TouchPhase::Moved, 1, [15.0, 7.0]) == Some(Gesture::Pan([5.0, -3.0])));
    }

    #[test]
    fn spreading_two_fingers_shrinks_the_view_about_their_midpoint() {
        let mut t = Touches::default();
        t.event(TouchPhase::Started, 1, [100.0, 100.0]);
        t.event(TouchPhase::Started, 2, [200.0, 100.0]);
        let gesture = t.event(TouchPhase::Moved, 2, [300.0, 100.0]);
        check!(gesture == Some(Gesture::Pinch { centre: [200.0, 100.0], pan: [50.0, 0.0], factor: 0.5 }));
    }

    #[test]
    fn a_third_finger_is_ignored_and_lifting_one_returns_to_panning() {
        let mut t = Touches::default();
        t.event(TouchPhase::Started, 1, [0.0, 0.0]);
        t.event(TouchPhase::Started, 2, [100.0, 0.0]);
        t.event(TouchPhase::Started, 3, [50.0, 50.0]);
        check!(t.event(TouchPhase::Moved, 3, [60.0, 60.0]).is_none());
        t.event(TouchPhase::Ended, 1, [0.0, 0.0]);
        check!(t.event(TouchPhase::Moved, 2, [110.0, 0.0]) == Some(Gesture::Pan([10.0, 0.0])));
    }

    #[test]
    fn a_finger_is_still_until_it_strays_past_the_slop_or_another_joins() {
        let mut t = Touches::default();
        t.event(TouchPhase::Started, 1, [100.0, 100.0]);
        t.event(TouchPhase::Moved, 1, [104.0, 103.0]);
        check!(t.still(10.0) == Some([104.0, 103.0]));
        t.event(TouchPhase::Started, 2, [300.0, 300.0]);
        check!(t.still(10.0).is_none());
        t.event(TouchPhase::Ended, 2, [300.0, 300.0]);
        t.event(TouchPhase::Moved, 1, [130.0, 100.0]);
        check!(t.still(10.0).is_none());
        check!(t.any());
        t.event(TouchPhase::Ended, 1, [130.0, 100.0]);
        check!(!t.any());
    }

    #[test]
    fn forgotten_fingers_move_nothing() {
        let mut t = Touches::default();
        t.event(TouchPhase::Started, 1, [0.0, 0.0]);
        t.forget();
        check!(t.event(TouchPhase::Moved, 1, [50.0, 0.0]).is_none());
        check!(!t.any());
    }
}
