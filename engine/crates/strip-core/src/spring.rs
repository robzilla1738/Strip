//! Critically and under-damped springs for strip motion.
//!
//! ω = 2π / (duration_s × 0.9). Integration is semi-implicit Euler with `dt` capped at 1/30 s.

use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub x: f64,
    pub v: f64,
    pub target: f64,
}

impl Spring {
    pub fn omega(duration_s: f64) -> f64 {
        if duration_s <= 0.0 {
            0.0
        } else {
            TAU / (duration_s * 0.9)
        }
    }

    pub fn launch(x: f64, target: f64) -> Self {
        Self { x, v: 0.0, target }
    }

    pub fn retarget(&mut self, target: f64) {
        self.target = target;
    }

    /// Returns true when the mass is at rest on the target.
    pub fn step(&mut self, dt: f64, zeta: f64, omega: f64) -> bool {
        let dt = dt.min(1.0 / 30.0);
        if omega <= 0.0 || dt <= 0.0 {
            self.x = self.target;
            self.v = 0.0;
            return true;
        }
        // Substep so a dropped frame integrates the same curve as a clean
        // 120/240 Hz cadence instead of ringing near the stability limit.
        let steps = (dt / (1.0 / 240.0)).ceil() as u32;
        let h = dt / f64::from(steps.max(1));
        for _ in 0..steps.max(1) {
            let accel = -2.0 * zeta * omega * self.v - omega * omega * (self.x - self.target);
            self.v += accel * h;
            let next = self.x + self.v * h;
            if (zeta - 1.0).abs() < 1e-9 && (self.x - self.target) * (next - self.target) < 0.0 {
                self.x = self.target;
                self.v = 0.0;
                return true;
            }
            self.x = next;
        }
        (self.x - self.target).abs() < 0.5 && self.v.abs() < 5.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_damping_never_overshoots() {
        let omega = Spring::omega(0.5);
        let mut spring = Spring::launch(0.0, 200.0);
        for _ in 0..240 {
            spring.step(1.0 / 120.0, 1.0, omega);
            assert!(spring.x <= 200.0 + 1e-6, "overshoot {}", spring.x);
            assert!(spring.x >= -1e-6, "negative {}", spring.x);
        }
    }

    #[test]
    fn underdamped_settles_within_the_duration_margin() {
        let duration = 0.5;
        let omega = Spring::omega(duration);
        let mut spring = Spring::launch(0.0, 400.0);
        let dt = 1.0 / 120.0;
        let mut elapsed = 0.0;
        let mut settled_at = None;
        for _ in 0..400 {
            elapsed += dt;
            if spring.step(dt, 0.9, omega) {
                settled_at = Some(elapsed);
                break;
            }
        }
        let settled_at = settled_at.expect("spring did not settle");
        assert!(settled_at <= duration * 1.2, "settled at {settled_at}");
    }

    #[test]
    fn retarget_keeps_velocity() {
        let omega = Spring::omega(0.5);
        let mut spring = Spring::launch(0.0, 100.0);
        spring.step(1.0 / 60.0, 0.9, omega);
        let velocity = spring.v;
        spring.retarget(20.0);
        assert_eq!(spring.v, velocity);
        assert_eq!(spring.target, 20.0);
    }

    #[test]
    fn dt_cap_matches_one_thirtieth() {
        let omega = Spring::omega(0.5);
        let mut open = Spring::launch(0.0, 100.0);
        let mut capped = Spring::launch(0.0, 100.0);
        open.step(1.0, 0.9, omega);
        capped.step(1.0 / 30.0, 0.9, omega);
        assert!((open.x - capped.x).abs() < 1e-9);
        assert!((open.v - capped.v).abs() < 1e-9);
    }
}
