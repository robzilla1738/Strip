//! Wake, unlock, display reconfiguration, and Space changes set flags.
#![allow(deprecated, unexpected_cfgs)]
//! The main thread runs the resync. Workspace notifications are not delivered
//! on the distributed center.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::{class, msg_send, sel, sel_impl};

use crate::hotkeys;

static REQUESTED: AtomicBool = AtomicBool::new(false);
static SPACE_CHANGED: AtomicBool = AtomicBool::new(false);
static WORKSPACE_CHANGED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

const DISTRIBUTED: &[&str] = &["com.apple.screenIsUnlocked", "com.apple.screenIsLocked"];

const WORKSPACE_WAKE: &[&str] = &[
    "NSWorkspaceDidWakeNotification",
    "NSWorkspaceScreensDidWakeNotification",
];

const WORKSPACE_SPACE: &[&str] = &["NSWorkspaceActiveSpaceDidChangeNotification"];

const WORKSPACE_APPS: &[&str] = &[
    "NSWorkspaceDidLaunchApplicationNotification",
    "NSWorkspaceDidTerminateApplicationNotification",
    "NSWorkspaceDidHideApplicationNotification",
    "NSWorkspaceDidUnhideApplicationNotification",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeCenter {
    Distributed,
    Workspace,
}

/// `com.apple.*` screen lock notices are distributed. NSWorkspace notices are not.
pub fn notice_center(name: &str) -> NoticeCenter {
    if name.starts_with("com.apple.") {
        NoticeCenter::Distributed
    } else {
        NoticeCenter::Workspace
    }
}

pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let center = CFNotificationCenterGetDistributedCenter();
        if !center.is_null() {
            for name in DISTRIBUTED {
                debug_assert_eq!(notice_center(name), NoticeCenter::Distributed);
                let Ok(text) = CString::new(*name) else {
                    continue;
                };
                let cf = CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x0800_0100);
                if cf.is_null() {
                    continue;
                }
                let callback = if *name == "com.apple.screenIsUnlocked" {
                    wake_callback
                } else {
                    ignore_callback
                };
                CFNotificationCenterAddObserver(center, std::ptr::null_mut(), callback, cf, std::ptr::null(), 4);
            }
        }
        for name in WORKSPACE_WAKE {
            debug_assert_eq!(notice_center(name), NoticeCenter::Workspace);
            observe_workspace(name, WorkspaceKind::Wake);
        }
        for name in WORKSPACE_SPACE {
            debug_assert_eq!(notice_center(name), NoticeCenter::Workspace);
            observe_workspace(name, WorkspaceKind::Space);
        }
        for name in WORKSPACE_APPS {
            debug_assert_eq!(notice_center(name), NoticeCenter::Workspace);
            observe_workspace(name, WorkspaceKind::Apps);
        }
        let _ = CGDisplayRegisterReconfigurationCallback(display_callback, std::ptr::null_mut());
    }
}

#[derive(Clone, Copy)]
enum WorkspaceKind {
    Wake,
    Space,
    Apps,
}

fn observe_workspace(name: &str, kind: WorkspaceKind) {
    unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return;
        }
        let center: id = msg_send![workspace, notificationCenter];
        if center.is_null() {
            return;
        }
        let label = NSString::alloc(nil).init_str(name);
        let block = block::ConcreteBlock::new(move |_note: id| {
            match kind {
                WorkspaceKind::Wake => REQUESTED.store(true, Ordering::SeqCst),
                WorkspaceKind::Space => SPACE_CHANGED.store(true, Ordering::SeqCst),
                WorkspaceKind::Apps => WORKSPACE_CHANGED.store(true, Ordering::SeqCst),
            }
            hotkeys::wake_main();
        });
        let block = block.copy();
        let _: id = msg_send![center, addObserverForName: label object: nil queue: nil usingBlock: &*block];
        std::mem::forget(block);
    }
}

pub fn note_ax_window_event() {
    WORKSPACE_CHANGED.store(true, Ordering::SeqCst);
    hotkeys::wake_main();
}

pub fn take_request() -> bool {
    REQUESTED.swap(false, Ordering::SeqCst)
}

pub fn take_space_change() -> bool {
    SPACE_CHANGED.swap(false, Ordering::SeqCst)
}

pub fn take_workspace_change() -> bool {
    WORKSPACE_CHANGED.swap(false, Ordering::SeqCst)
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

extern "C" fn ignore_callback(
    _center: *mut c_void,
    _observer: *mut c_void,
    _name: *const c_void,
    _object: *const c_void,
    _info: *const c_void,
) {
}

extern "C" fn display_callback(_display: u32, flags: u32, _user: *mut c_void) {
    const BEGIN: u32 = 1;
    if flags & BEGIN != 0 {
        return;
    }
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

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDisplayRegisterReconfigurationCallback(
        callback: extern "C" fn(u32, u32, *mut c_void),
        user: *mut c_void,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_wake_is_not_a_distributed_notification() {
        assert_eq!(notice_center("NSWorkspaceDidWakeNotification"), NoticeCenter::Workspace);
        assert_eq!(notice_center("NSWorkspaceScreensDidWakeNotification"), NoticeCenter::Workspace);
        assert_eq!(notice_center("NSWorkspaceActiveSpaceDidChangeNotification"), NoticeCenter::Workspace);
        assert_eq!(notice_center("com.apple.screenIsUnlocked"), NoticeCenter::Distributed);
        assert_eq!(notice_center("com.apple.screenIsLocked"), NoticeCenter::Distributed);
    }
}
