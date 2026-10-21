//! Parking repair and the on-screen frame used when managing stops.

use strip_core::{parked_corner, Corner, DisplayFrame, Frame, Side};

/// After a park, if the window server moved the window onto another display,
/// flip the corner and shrink the exposed edge to 1 pt.
pub fn repair_relocated(relocated: bool, corner: Corner) -> (Corner, f64) {
    if !relocated {
        return (corner, 0.0);
    }
    let flipped = match corner {
        Corner::BottomLeft => Corner::BottomRight,
        Corner::BottomRight => Corner::BottomLeft,
    };
    (flipped, 1.0)
}

/// The display that owns the parked frame differs from the display that owns
/// the read-back center. Flip the corner and expose 1 pt.
pub fn relocated_repair(home: &str, landed: &str, corner: Corner) -> Option<(Corner, f64)> {
    if home.is_empty() || home == landed {
        None
    } else {
        Some(repair_relocated(true, corner))
    }
}

pub fn choose_park(side: Side, display: DisplayFrame, gap_outer: f64, sliver: f64) -> (Corner, f64) {
    parked_corner(side, display.neighbour_left, display.neighbour_right, gap_outer, sliver)
}

/// Last fully on-screen frame, or a cascade inside the visible frame.
pub fn restored_frame(last_on_screen: Option<Frame>, visible: Frame, cascade: usize) -> Frame {
    if let Some(frame) = last_on_screen.filter(|frame| contains(visible, *frame)) {
        return frame;
    }
    let shift = 24.0 * cascade as f64;
    let width = 520.0_f64.min((visible.width - 16.0).max(1.0));
    let height = 320.0_f64.min((visible.height - 16.0).max(1.0));
    let max_x = visible.x + (visible.width - width).max(0.0);
    let max_y = visible.y + (visible.height - height).max(0.0);
    Frame {
        x: (visible.x + 8.0 + shift).min(max_x),
        y: (visible.y + 8.0 + shift).min(max_y),
        width,
        height,
    }
}

fn contains(outer: Frame, inner: Frame) -> bool {
    inner.width > 1.0
        && inner.height > 1.0
        && inner.x >= outer.x - 0.5
        && inner.y >= outer.y - 0.5
        && inner.right() <= outer.right() + 0.5
        && inner.bottom() <= outer.bottom() + 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible() -> Frame {
        Frame {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 800.0,
        }
    }

    #[test]
    fn restore_prefers_the_last_on_screen_frame() {
        let last = Frame {
            x: 40.0,
            y: 50.0,
            width: 600.0,
            height: 400.0,
        };
        assert_eq!(restored_frame(Some(last), visible(), 0), last);
    }

    #[test]
    fn restore_cascades_a_parked_frame_back_inside() {
        let parked = Frame {
            x: 1439.0,
            y: 799.0,
            width: 500.0,
            height: 400.0,
        };
        let restored = restored_frame(Some(parked), visible(), 1);
        assert!(restored.x >= 0.0 && restored.right() <= 1440.0);
        assert!(restored.y >= 0.0 && restored.bottom() <= 800.0);
    }

    #[test]
    fn relocated_park_flips_corner_and_uses_one_point() {
        let (corner, sliver) = repair_relocated(true, Corner::BottomRight);
        assert_eq!(corner, Corner::BottomLeft);
        assert_eq!(sliver, 1.0);
        let (corner, sliver) = repair_relocated(false, Corner::BottomRight);
        assert_eq!(corner, Corner::BottomRight);
        assert_eq!(sliver, 0.0);
    }

    #[test]
    fn a_read_back_on_another_display_flips_the_parked_corner() {
        let repaired = relocated_repair("display-a", "display-b", Corner::BottomLeft);
        assert_eq!(repaired, Some((Corner::BottomRight, 1.0)));
        assert_eq!(relocated_repair("display-a", "display-a", Corner::BottomLeft), None);
    }
}
