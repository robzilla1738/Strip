//! Trackpad swipe math. No macOS types.
//!
//! Δscroll = −Δ(mean normalized x) × sensitivity × viewport width when `natural` is set.
//! The opposite sign is used otherwise. Past the first or last column the extra travel is scaled by 0.3.

const RECOGNITION_PT: f64 = 4.0;
/// Diagonal travel stays undecided until one axis dominates or this much
/// travel forces a decision by the larger axis.
const DECISION_PT: f64 = 14.0;
const DOMINANCE: f64 = 1.5;
const RUBBER: f64 = 0.3;
/// Velocity smoothing time constant, so the estimate does not depend on the
/// trackpad's event rate.
const VELOCITY_TAU_S: f64 = 0.05;
const MAX_VELOCITY: f64 = 12_000.0;
const REST_SPEED: f64 = 5.0;
const FLICK_SPEED: f64 = 300.0;
const FLICK_MIN_TRAVEL_PT: f64 = 20.0;

#[derive(Clone, Debug)]
pub struct SwipeView {
    pub viewport_width: f64,
    pub sensitivity: f64,
    pub natural: bool,
    pub deceleration: f64,
    pub spans: Vec<(f64, f64)>,
    /// Width the columns scroll through: the viewport less outer gaps and pins.
    pub view_width: f64,
    /// Snap stops center a column instead of lining up one of its edges.
    pub center: bool,
}

impl SwipeView {
    pub fn from_ratios(
        ratios: &[f64],
        viewport_width: f64,
        gap_inner: f64,
        sensitivity: f64,
        natural: bool,
        deceleration: f64,
    ) -> Self {
        let mut spans = Vec::with_capacity(ratios.len());
        let mut cursor = 0.0;
        for (index, ratio) in ratios.iter().enumerate() {
            let width = ratio * viewport_width;
            spans.push((cursor, width));
            cursor += width;
            if index + 1 != ratios.len() {
                cursor += gap_inner;
            }
        }
        Self {
            viewport_width,
            sensitivity,
            natural,
            deceleration,
            spans,
            view_width: viewport_width,
            center: false,
        }
    }

    pub fn with_view(mut self, view_width: f64, center: bool) -> Self {
        self.view_width = view_width;
        self.center = center;
        self
    }

    pub fn with_spans(mut self, spans: Vec<(f64, f64)>) -> Self {
        self.spans = spans;
        self
    }

    pub fn limits(&self) -> (f64, f64) {
        let content = self.spans.last().map(|(prefix, width)| prefix + width).unwrap_or(0.0);
        (0.0, (content - self.view_width).max(0.0))
    }
}

#[derive(Clone, Debug)]
pub struct Gesture {
    open: bool,
    ignored: bool,
    armed: bool,
    origin: f64,
    accum_x: f64,
    accum_y: f64,
    /// `accum_x` at the moment the swipe armed; travel before arming is the
    /// recognition dead zone and must not jump the strip.
    arm_x: f64,
    velocity: f64,
    primed: bool,
    scroll: f64,
}

impl Default for Gesture {
    fn default() -> Self {
        Self::new()
    }
}

impl Gesture {
    pub fn new() -> Self {
        Self {
            open: false,
            ignored: false,
            armed: false,
            origin: 0.0,
            accum_x: 0.0,
            accum_y: 0.0,
            arm_x: 0.0,
            velocity: 0.0,
            primed: false,
            scroll: 0.0,
        }
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Drops the tracked velocity, so a release after a pause snaps without a fling.
    pub fn rest(&mut self) {
        self.velocity = 0.0;
    }

    /// Smoothed scroll velocity of the last tracked frames, in points per second.
    pub fn velocity(&self) -> f64 {
        self.velocity
    }

    pub fn begin(&mut self, fingers: u8, wanted: u8, scroll: f64) {
        self.open = true;
        self.ignored = wanted == 0 || fingers != wanted;
        self.armed = false;
        self.origin = scroll;
        self.scroll = scroll;
        self.accum_x = 0.0;
        self.accum_y = 0.0;
        self.arm_x = 0.0;
        self.velocity = 0.0;
        self.primed = false;
    }

    pub fn moved(&mut self, dx: f64, dy: f64, view: &SwipeView, dt: f64) -> SwipeUpdate {
        if !self.open || self.ignored {
            return SwipeUpdate::default();
        }
        self.accum_x += dx;
        self.accum_y += dy;
        let travel_x = self.accum_x.abs() * view.viewport_width;
        let travel_y = self.accum_y.abs() * view.viewport_width;
        if !self.armed {
            let travel = travel_x.hypot(travel_y);
            if travel < RECOGNITION_PT {
                return SwipeUpdate::default();
            }
            let horizontal = travel_x >= travel_y * DOMINANCE;
            let vertical = travel_y >= travel_x * DOMINANCE;
            if !horizontal && !vertical && travel < DECISION_PT {
                // Diagonal so far: keep watching instead of killing the swipe.
                return SwipeUpdate::default();
            }
            if vertical || (!horizontal && travel_y > travel_x) {
                self.ignored = true;
                return SwipeUpdate::default();
            }
            self.armed = true;
            self.arm_x = self.accum_x;
        }
        let delta = tracking_delta(self.accum_x - self.arm_x, view);
        let (min, max) = view.limits();
        let next = rubber_band(self.origin + delta, min, max);
        let step_pt = (dx * view.viewport_width).abs();
        if dt > 0.0 && (step_pt >= 0.25 || !self.primed || dt >= 0.04) {
            let instant = (tracking_delta(dx, view) / dt).clamp(-MAX_VELOCITY, MAX_VELOCITY);
            self.velocity = if self.primed {
                let blend = 1.0 - (-dt / VELOCITY_TAU_S).exp();
                blend * instant + (1.0 - blend) * self.velocity
            } else {
                instant
            };
            self.primed = true;
        }
        self.scroll = next;
        SwipeUpdate { scroll: Some(next), focus: None }
    }

    pub fn ended(&mut self, view: &SwipeView) -> SwipeUpdate {
        if !self.open {
            return SwipeUpdate::default();
        }
        self.open = false;
        if self.ignored || !self.armed {
            return SwipeUpdate::default();
        }
        let (min, max) = view.limits();
        // Exponential decay travels velocity / deceleration in total; project
        // the landing point analytically instead of stepping a rubber band.
        let landing = if self.velocity.abs() > REST_SPEED {
            self.scroll + self.velocity / view.deceleration.max(0.1)
        } else {
            self.scroll
        }
        .clamp(min, max);
        let (mut snapped, mut index) = snap_column(landing, &view.spans, view.view_width, view.center);
        let net = self.scroll - self.origin;
        if self.velocity.abs() >= FLICK_SPEED
            && net.abs() >= FLICK_MIN_TRAVEL_PT
            && net * self.velocity > 0.0
            && (snapped - self.scroll) * self.velocity < -1.0
        {
            if let Some(forward) = snap_column_directed(self.scroll, self.velocity, &view.spans, view.view_width, view.center) {
                (snapped, index) = forward;
            }
        }
        self.scroll = snapped;
        SwipeUpdate { scroll: Some(snapped), focus: Some(index) }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwipeUpdate {
    pub scroll: Option<f64>,
    pub focus: Option<usize>,
}

pub fn tracking_delta(dx: f64, view: &SwipeView) -> f64 {
    let sign = if view.natural { -1.0 } else { 1.0 };
    sign * dx * view.sensitivity * view.viewport_width
}

pub fn rubber_band(scroll: f64, min: f64, max: f64) -> f64 {
    let (min, max) = if max < min { (max, min) } else { (min, max) };
    if scroll < min {
        min + (scroll - min) * RUBBER
    } else if scroll > max {
        max + (scroll - max) * RUBBER
    } else {
        scroll
    }
}

pub fn decay_velocity(velocity: f64, deceleration: f64, dt: f64) -> f64 {
    velocity * (-deceleration * dt).exp()
}

/// The nearest scroll stop and the column it lands on. Without centering, a
/// stop lines up a column's left edge with the viewport's left edge or its right
/// edge with the viewport's right edge, so whole columns show, as the keyboard
/// leaves them.
pub fn snap_column(scroll: f64, spans: &[(f64, f64)], viewport: f64, center: bool) -> (f64, usize) {
    if spans.is_empty() || viewport <= 0.0 {
        return (0.0, 0);
    }
    let content = spans.last().map(|(prefix, width)| prefix + width).unwrap_or(0.0);
    let max = (content - viewport).max(0.0);
    let mut best_scroll = 0.0;
    let mut best_index = 0;
    let mut best_distance = f64::MAX;
    for (index, (prefix, width)) in spans.iter().enumerate() {
        let stops = if center {
            [Some(prefix + width / 2.0 - viewport / 2.0), None]
        } else {
            [Some(*prefix), Some(prefix + width - viewport)]
        };
        for stop in stops.into_iter().flatten() {
            let target = stop.clamp(0.0, max);
            let distance = (target - scroll).abs();
            if distance < best_distance - 1e-9 {
                best_distance = distance;
                best_scroll = target;
                best_index = index;
            }
        }
    }
    (best_scroll, best_index)
}

fn snap_column_directed(
    scroll: f64,
    velocity: f64,
    spans: &[(f64, f64)],
    viewport: f64,
    center: bool,
) -> Option<(f64, usize)> {
    if spans.is_empty() || viewport <= 0.0 {
        return None;
    }
    let content = spans.last().map(|(prefix, width)| prefix + width).unwrap_or(0.0);
    let max = (content - viewport).max(0.0);
    let mut best: Option<(f64, usize, f64)> = None;
    for (index, (prefix, width)) in spans.iter().enumerate() {
        let stops = if center {
            [Some(prefix + width / 2.0 - viewport / 2.0), None]
        } else {
            [Some(*prefix), Some(prefix + width - viewport)]
        };
        for stop in stops.into_iter().flatten() {
            let target = stop.clamp(0.0, max);
            let delta = target - scroll;
            if delta * velocity <= 1.0 {
                continue;
            }
            let distance = delta.abs();
            if best.is_none_or(|(_, _, best_dist)| distance < best_dist - 1e-9) {
                best = Some((target, index, distance));
            }
        }
    }
    best.map(|(target, index, _)| (target, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(width: f64) -> SwipeView {
        SwipeView::from_ratios(&[0.5, 0.5, 0.5, 0.5], width, 8.0, 1.0, true, 4.0)
    }

    #[test]
    fn tracking_is_one_to_one_with_natural_direction() {
        let view = view(1000.0);
        let mut gesture = Gesture::new();
        gesture.begin(4, 4, 20.0);
        // The arming frame only crosses the recognition dead zone.
        let armed = gesture.moved(-0.02, 0.0, &view, 1.0 / 60.0);
        assert!((armed.scroll.expect("armed") - 20.0).abs() < 1e-6);
        let update = gesture.moved(-0.48, 0.0, &view, 1.0 / 60.0);
        let scroll = update.scroll.expect("tracking");
        assert!((scroll - (20.0 + 480.0)).abs() < 1e-6, "scroll {scroll}");
    }

    #[test]
    fn a_diagonal_start_can_still_become_a_swipe() {
        let view = view(1000.0);
        let mut gesture = Gesture::new();
        gesture.begin(4, 4, 0.0);
        let diagonal = gesture.moved(-0.010, 0.009, &view, 1.0 / 60.0);
        assert!(diagonal.scroll.is_none());
        assert!(!gesture.is_armed());
        gesture.moved(-0.02, 0.0, &view, 1.0 / 60.0);
        assert!(gesture.is_armed());
        let update = gesture.moved(-0.1, 0.0, &view, 1.0 / 60.0);
        assert!(update.scroll.expect("tracking") > 50.0);
    }

    #[test]
    fn fling_decay_matches_the_exponential() {
        let decayed = decay_velocity(80.0, 4.0, 0.25);
        assert!((decayed - 80.0 * (-1.0_f64).exp()).abs() < 1e-9);
    }

    #[test]
    fn snap_picks_the_column_center_nearest_the_viewport() {
        let spans = [(0.0, 400.0), (408.0, 400.0), (816.0, 400.0)];
        let (scroll, index) = snap_column(180.0, &spans, 800.0, true);
        assert_eq!(index, 1);
        let center = 408.0 + 200.0 - 400.0;
        assert!((scroll - center).abs() < 1e-6, "scroll {scroll}");
    }

    #[test]
    fn snap_without_centering_shows_whole_columns() {
        let spans = [(0.0, 400.0), (408.0, 400.0), (816.0, 400.0), (1224.0, 400.0)];
        let (scroll, index) = snap_column(380.0, &spans, 800.0, false);
        assert_eq!((scroll, index), (408.0, 1));
        let (end, last) = snap_column(900.0, &spans, 800.0, false);
        assert_eq!((end, last), (824.0, 3));
    }

    #[test]
    fn rubber_band_scales_travel_past_the_ends() {
        assert!((rubber_band(-20.0, 0.0, 100.0) - (-6.0)).abs() < 1e-9);
        assert!((rubber_band(130.0, 0.0, 100.0) - 109.0).abs() < 1e-9);
        assert!((rubber_band(40.0, 0.0, 100.0) - 40.0).abs() < 1e-9);
    }

    #[test]
    fn reversing_direction_keeps_tracking() {
        let view = view(1000.0);
        let mut gesture = Gesture::new();
        gesture.begin(4, 4, 0.0);
        gesture.moved(0.02, 0.0, &view, 1.0 / 60.0);
        let forward = gesture.moved(0.18, 0.0, &view, 1.0 / 60.0).scroll.unwrap();
        let back = gesture.moved(-0.35, 0.0, &view, 1.0 / 60.0).scroll.unwrap();
        assert!(forward < -1.0, "forward {forward}");
        assert!(back > forward, "back {back} forward {forward}");
    }

    #[test]
    fn a_short_vertical_travel_does_not_arm() {
        let view = view(1000.0);
        let mut gesture = Gesture::new();
        gesture.begin(4, 4, 10.0);
        let update = gesture.moved(0.002, 0.02, &view, 1.0 / 60.0);
        assert!(update.scroll.is_none());
    }
}
