#![allow(deprecated, unexpected_cfgs)]

//! Gesture events stay in the tap stream. This module only observes them.

use std::cell::RefCell;
use std::os::raw::c_void;

use cocoa::base::id;
use cocoa::foundation::NSPoint;
use objc::{class, msg_send, sel, sel_impl};

use crate::session::Runtime;

const GESTURE: u32 = 29;

struct Anchor {
    id: u64,
    x: f64,
    y: f64,
}

thread_local! {
    static ANCHORS: RefCell<Vec<Anchor>> = const { RefCell::new(Vec::new()) };
}

pub fn observe(runtime: &mut Runtime, event_type: u32, event: *mut c_void) -> crate::session::SwipeNotice {
    if event_type != GESTURE || event.is_null() {
        return crate::session::SwipeNotice { started: false, updated: false };
    }
    let Some(frame) = read_frame(event) else {
        return crate::session::SwipeNotice { started: false, updated: false };
    };
    let armed = runtime.swipe_armed();
    let (dx, dy, phase) = ANCHORS.with(|anchors| step_anchors(&mut anchors.borrow_mut(), &frame, armed));
    runtime.drive_swipe(frame.fingers, dx, dy, phase)
}

struct Finger {
    id: u64,
    x: f64,
    y: f64,
    began: bool,
    ended: bool,
}

struct Frame {
    fingers: u8,
    fingers_detail: Vec<Finger>,
    event_ended: bool,
}

/// A touch that lands once the swipe is armed (a resting thumb or palm) joins
/// the stream instead of restarting it, so the strip never stops mid-swipe.
/// Likewise one finger lifting early (fingers rarely leave together) ends
/// nothing: the swipe ends when the event ends or no touch remains down.
fn step_anchors(anchors: &mut Vec<Anchor>, frame: &Frame, armed: bool) -> (f64, f64, &'static str) {
    let active: Vec<&Finger> = frame.fingers_detail.iter().filter(|finger| !finger.ended).collect();
    let ended = frame.event_ended || (!frame.fingers_detail.is_empty() && (active.is_empty() || (armed && active.len() <= 1)));
    if ended {
        anchors.clear();
        return (0.0, 0.0, "ended");
    }
    // AppKit interleaves gesture sub-events whose touch set is empty between
    // real touch samples; they must not clear anchors or restart the swipe.
    if active.is_empty() {
        return (0.0, 0.0, "moved");
    }
    let landed = active.iter().any(|finger| finger.began || !anchors.iter().any(|anchor| anchor.id == finger.id));
    let began = anchors.is_empty() || (!armed && (landed || active.len() > anchors.len()));
    if began {
        *anchors = active.iter().map(|finger| Anchor { id: finger.id, x: finger.x, y: finger.y }).collect();
        return (0.0, 0.0, "began");
    }
    // Median across fingers: one noisy finger cannot jitter the strip.
    let mut deltas_x: Vec<f64> = Vec::with_capacity(active.len());
    let mut deltas_y: Vec<f64> = Vec::with_capacity(active.len());
    for finger in &active {
        if let Some(anchor) = anchors.iter().find(|anchor| anchor.id == finger.id) {
            deltas_x.push(finger.x - anchor.x);
            deltas_y.push(finger.y - anchor.y);
        }
    }
    *anchors = active.iter().map(|finger| Anchor { id: finger.id, x: finger.x, y: finger.y }).collect();
    (median(&mut deltas_x), median(&mut deltas_y), "moved")
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

/// The tap runs outside AppKit's event loop, so nothing drains autoreleased
/// objects for us; each gesture frame gets its own pool.
fn read_frame(event: *mut c_void) -> Option<Frame> {
    unsafe {
        let pool: id = msg_send![class!(NSAutoreleasePool), new];
        let frame = read_frame_in_pool(event);
        let _: () = msg_send![pool, drain];
        frame
    }
}

unsafe fn read_frame_in_pool(event: *mut c_void) -> Option<Frame> {
    unsafe {
        let ns_event: id = msg_send![class!(NSEvent), eventWithCGEvent: event];
        if ns_event.is_null() {
            return None;
        }
        let kind: usize = msg_send![ns_event, type];
        if kind != GESTURE as usize {
            return None;
        }
        let event_phase: usize = msg_send![ns_event, phase];
        let event_ended = event_phase & (8 | 16) != 0;
        let touches: id = msg_send![ns_event, allTouches];
        if touches.is_null() {
            return event_ended.then_some(Frame {
                fingers: 0,
                fingers_detail: Vec::new(),
                event_ended: true,
            });
        }
        let count: usize = msg_send![touches, count];
        let objects: id = msg_send![touches, allObjects];
        let mut fingers_detail = Vec::with_capacity(count);
        if !objects.is_null() {
            for index in 0..count {
                let touch: id = msg_send![objects, objectAtIndex: index];
                if touch.is_null() {
                    continue;
                }
                let point: NSPoint = msg_send![touch, normalizedPosition];
                let phase: usize = msg_send![touch, phase];
                let identity: id = msg_send![touch, identity];
                let id_hash: usize = if identity.is_null() { index } else { msg_send![identity, hash] };
                fingers_detail.push(Finger {
                    id: id_hash as u64,
                    x: point.x,
                    y: point.y,
                    began: phase == 1,
                    ended: phase == 8 || phase == 16,
                });
            }
        }
        if !event_ended && fingers_detail.is_empty() {
            return None;
        }
        let active_count = fingers_detail.iter().filter(|finger| !finger.ended).count();
        Some(Frame {
            fingers: active_count.min(255) as u8,
            fingers_detail,
            event_ended,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finger(id: u64, x: f64, began: bool) -> Finger {
        Finger { id, x, y: 0.5, began, ended: false }
    }

    fn frame(fingers: Vec<Finger>) -> Frame {
        Frame { fingers: fingers.len() as u8, fingers_detail: fingers, event_ended: false }
    }

    #[test]
    fn a_touch_landing_mid_swipe_does_not_restart_it() {
        let mut anchors = Vec::new();
        let start = frame((1..=4).map(|id| finger(id, 0.5, true)).collect());
        assert_eq!(step_anchors(&mut anchors, &start, false).2, "began");
        let mut palm: Vec<Finger> = (1..=4).map(|id| finger(id, 0.4, false)).collect();
        palm.push(finger(9, 0.9, true));
        let (dx, _, phase) = step_anchors(&mut anchors, &frame(palm), true);
        assert_eq!(phase, "moved");
        assert!((dx + 0.1).abs() < 1e-9, "dx {dx}");
    }

    #[test]
    fn one_finger_lifting_early_does_not_end_the_swipe() {
        let mut anchors = Vec::new();
        step_anchors(&mut anchors, &frame((1..=4).map(|id| finger(id, 0.5, true)).collect()), false);
        let mut lifting: Vec<Finger> = (1..=3).map(|id| finger(id, 0.4, false)).collect();
        lifting.push(Finger { ended: true, ..finger(4, 0.4, false) });
        let (dx, _, phase) = step_anchors(&mut anchors, &frame(lifting), true);
        assert_eq!(phase, "moved");
        assert!((dx + 0.1).abs() < 1e-9, "dx {dx}");
        let all_up: Vec<Finger> = (1..=3).map(|id| Finger { ended: true, ..finger(id, 0.4, false) }).collect();
        assert_eq!(step_anchors(&mut anchors, &frame(all_up), true).2, "ended");
    }

    #[test]
    fn a_touch_landing_before_arming_restarts() {
        let mut anchors = Vec::new();
        step_anchors(&mut anchors, &frame((1..=3).map(|id| finger(id, 0.5, true)).collect()), false);
        let mut more: Vec<Finger> = (1..=3).map(|id| finger(id, 0.5, false)).collect();
        more.push(finger(4, 0.5, true));
        assert_eq!(step_anchors(&mut anchors, &frame(more), false).2, "began");
    }
}
