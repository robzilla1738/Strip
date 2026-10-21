//! Wake and unlock notifications set a flag. The main thread runs the resync.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::hotkeys;

static REQUESTED: AtomicBool = AtomicBool::new(false);

const NAMES: &[&str] = &[
    "com.apple.screenIsUnlocked",
    "NSWorkspaceDidWakeNotification",
    "NSWorkspaceScreensDidWakeNotification",
];

pub fn install() {
    unsafe {
        let center = CFNotificationCenterGetDistributedCenter();
        if center.is_null() {
            return;
        }
        for name in NAMES {
            let Ok(text) = CString::new(*name) else {
                continue;
            };
            let cf = CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x0800_0100);
            if cf.is_null() {
                continue;
            }
            CFNotificationCenterAddObserver(center, std::ptr::null_mut(), wake_callback, cf, std::ptr::null(), 4);
        }
    }
}

pub fn take_request() -> bool {
    REQUESTED.swap(false, Ordering::SeqCst)
}

extern "C" fn wake_callback(
    _center: *mut c_void,
    _observer: *mut c_void,
    _name: *const c_void,
    _object: *const c_void,
    _info: *const c_void,
) {
    REQUESTED.store(true, Ordering::SeqCst);
    hotkeys::wake_main();
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
    fn CFNotificationCenterAddObserver(
        center: *mut c_void,
        observer: *mut c_void,
        callback: extern "C" fn(*mut c_void, *mut c_void, *const c_void, *const c_void, *const c_void),
        name: *const c_void,
        object: *const c_void,
        suspension: i32,
    );
    fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
}
