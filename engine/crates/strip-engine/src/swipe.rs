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

pub fn observe(runtime: &mut Runtime, event_type: u32, event: *mut c_void) {
    if event_type != GESTURE || event.is_null() {
        return;
    }
    let Some(frame) = read_frame(event) else {
        return;
    };
    let (dx, dy, phase) = ANCHORS.with(|anchors| step_anchors(&mut anchors.borrow_mut(), &frame));
    runtime.drive_swipe(frame.fingers, dx, dy, phase);
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

fn step_anchors(anchors: &mut Vec<Anchor>, frame: &Frame) -> (f64, f64, &'static str) {
    let began = frame.fingers_detail.iter().any(|finger| finger.began) || anchors.is_empty();
    let ended = frame.event_ended || frame.fingers_detail.iter().any(|finger| finger.ended);
    if began && !ended {
        *anchors = frame
            .fingers_detail
            .iter()
            .map(|finger| Anchor { id: finger.id, x: finger.x, y: finger.y })
            .collect();
        return (0.0, 0.0, "began");
    }
    if ended {
        anchors.clear();
        return (0.0, 0.0, "ended");
    }
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut matched = 0.0;
    for finger in &frame.fingers_detail {
        if let Some(anchor) = anchors.iter().find(|anchor| anchor.id == finger.id) {
            sum_x += finger.x - anchor.x;
            sum_y += finger.y - anchor.y;
            matched += 1.0;
        }
    }
    *anchors = frame
        .fingers_detail
        .iter()
        .map(|finger| Anchor { id: finger.id, x: finger.x, y: finger.y })
        .collect();
    if matched == 0.0 {
        (0.0, 0.0, "moved")
    } else {
        (sum_x / matched, sum_y / matched, "moved")
    }
}

fn read_frame(event: *mut c_void) -> Option<Frame> {
    unsafe {
        let ns_event: id = msg_send![class!(NSEvent), eventWithCGEvent: event];
        if ns_event.is_null() {
            return None;
        }
        let event_phase: usize = msg_send![ns_event, phase];
        let touches: id = msg_send![ns_event, allTouches];
        if touches.is_null() {
            return None;
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
                fingers_detail.push(Finger {
                    id: identity as u64,
                    x: point.x,
                    y: point.y,
                    began: phase == 1,
                    ended: phase == 8 || phase == 16,
                });
            }
        }
        Some(Frame {
            fingers: fingers_detail.len().min(255) as u8,
            fingers_detail,
            event_ended: event_phase & (8 | 16) != 0,
        })
    }
}
