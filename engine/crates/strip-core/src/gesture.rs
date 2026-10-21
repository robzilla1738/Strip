//! Trackpad swipe math. No macOS types.
//!
//! Δscroll = −Δ(mean normalized x) × sensitivity × viewport width when `natural` is set.
//! The opposite sign is used otherwise. Past the first or last column the extra travel is scaled by 0.3.

const RECOGNITION_PT: f64 = 8.0;
const DOMINANCE: f64 = 1.5;
const RUBBER: f64 = 0.3;
const VELOCITY_BLEND: f64 = 0.3;
const REST_SPEED: f64 = 5.0;

#[derive(Clone, Debug)]
pub struct SwipeView {
    pub viewport_width: f64,
    pub sensitivity: f64,
    pub natural: bool,
    pub deceleration: f64,
    pub spans: Vec<(f64, f64)>,
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
        }
    }

    pub fn limits(&self) -> (f64, f64) {
        let content = self.spans.last().map(|(prefix, width)| prefix + width).unwrap_or(0.0);
        (0.0, (content - self.viewport_width).max(0.0))
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
            velocity: 0.0,
            primed: false,
            scroll: 0.0,
        }
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    pub fn begin(&mut self, fingers: u8, wanted: u8, scroll: f64) {
        self.open = true;
        self.ignored = wanted == 0 || fingers != wanted;
        self.armed = false;
        self.origin = scroll;
        self.scroll = scroll;
        self.accum_x = 0.0;
        self.accum_y = 0.0;
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
            if travel_x.hypot(travel_y) < RECOGNITION_PT {
                return SwipeUpdate::default();
            }
            if travel_x <= travel_y * DOMINANCE {
                self.ignored = true;
                return SwipeUpdate::default();
            }
            self.armed = true;
        }
        let delta = tracking_delta(self.accum_x, view);
        let (min, max) = view.limits();
        let next = rubber_band(self.origin + delta, min, max);
        if dt > 0.0 {
            let instant = tracking_delta(dx, view) / dt;
            self.velocity = if self.primed {
                VELOCITY_BLEND * instant + (1.0 - VELOCITY_BLEND) * self.velocity
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
        let mut scroll = self.scroll;
        let mut velocity = self.velocity;
        let mut elapsed = 0.0;
        while elapsed < 1.5 && velocity.abs() > REST_SPEED {
            let dt = 1.0 / 60.0;
            velocity = decay_velocity(velocity, view.deceleration, dt);
            scroll = rubber_band(scroll + velocity * dt, min, max);
            elapsed += dt;
        }
        let (snapped, index) = snap_column(scroll, &view.spans, view.viewport_width);
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

pub fn snap_column(scroll: f64, spans: &[(f64, f64)], viewport: f64) -> (f64, usize) {
    if spans.is_empty() || viewport <= 0.0 {
        return (0.0, 0);
    }
    let content = spans.last().map(|(prefix, width)| prefix + width).unwrap_or(0.0);
    let max = (content - viewport).max(0.0);
    let mut best_scroll = 0.0;
    let mut best_index = 0;
    let mut best_distance = f64::MAX;
    for (index, (prefix, width)) in spans.iter().enumerate() {
        let target = (prefix + width / 2.0 - viewport / 2.0).clamp(0.0, max);
        let distance = (target - scroll).abs();
        if distance < best_distance {
            best_distance = distance;
            best_scroll = target;
            best_index = index;
        }
    }
    (best_scroll, best_index)
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
        let update = gesture.moved(-0.5, 0.0, &view, 1.0 / 60.0);
        let scroll = update.scroll.expect("tracking");
        assert!((scroll - (20.0 + 500.0)).abs() < 1e-6, "scroll {scroll}");
    }

    #[test]
    fn fling_decay_matches_the_exponential() {
        let decayed = decay_velocity(80.0, 4.0, 0.25);
        assert!((decayed - 80.0 * (-1.0_f64).exp()).abs() < 1e-9);
    }

    #[test]
    fn snap_picks_the_column_center_nearest_the_viewport() {
        let spans = [(0.0, 400.0), (408.0, 400.0), (816.0, 400.0)];
        let (scroll, index) = snap_column(180.0, &spans, 800.0);
        assert_eq!(index, 1);
        let center = 408.0 + 200.0 - 400.0;
        assert!((scroll - center).abs() < 1e-6, "scroll {scroll}");
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
        let forward = gesture.moved(0.2, 0.0, &view, 1.0 / 60.0).scroll.unwrap();
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
