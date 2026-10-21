//! One HID event tap. Matched key-downs are swallowed and run on this thread.

use std::cell::Cell;
use std::os::raw::{c_void, c_uint};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};


use crate::bindings::Binding;
use crate::bus::Bus;
use crate::command;
use crate::session::Runtime;
use crate::swipe;

const KEY_DOWN: u32 = 10;
const FLAGS_CHANGED: u32 = 12;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;
const KEYCODE_FIELD: u32 = 9;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_ALTERNATE: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

thread_local! {
    static CONTEXT: Cell<*mut TapContext> = const { Cell::new(ptr::null_mut()) };
    static TAP_PORT: Cell<*mut c_void> = const { Cell::new(ptr::null_mut()) };
}

static MAIN_LOOP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

pub fn remember_main_loop() {
    let run_loop = unsafe { CFRunLoopGetMain() };
    MAIN_LOOP.store(run_loop, Ordering::SeqCst);
}

pub fn main_loop() -> *mut c_void {
    MAIN_LOOP.load(Ordering::SeqCst)
}

pub fn wake_main() {
    let run_loop = MAIN_LOOP.load(Ordering::SeqCst);
    if !run_loop.is_null() {
        unsafe { CFRunLoopWakeUp(run_loop) };
    }
}

struct TapContext {
    runtime: *mut Runtime,
    bus: *const Bus,
}

pub struct EventTap {
    port: *mut c_void,
    source: *mut c_void,
}

impl EventTap {
    pub fn alive(&self) -> bool {
        !self.port.is_null() && !self.source.is_null()
    }
}

/// Re-enable the tap created at startup. A second tap would swallow each key twice.
pub fn enable_tap() -> bool {
    let port = TAP_PORT.with(|cell| cell.get());
    if port.is_null() {
        eprintln!("event tap unavailable");
        return false;
    }
    unsafe { CGEventTapEnable(port, 1) };
    eprintln!("event tap enabled");
    true
}

pub fn install() -> Option<EventTap> {
    let mask = event_mask();
    let port = unsafe {
        CGEventTapCreate(0, 0, 0, mask, tap_callback, ptr::null_mut())
    };
    if port.is_null() {
        eprintln!("event tap unavailable");
        return None;
    }
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), port, 0) };
    if source.is_null() {
        eprintln!("event tap source unavailable");
        return None;
    }
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, 1);
    }
    TAP_PORT.with(|cell| cell.set(port));
    Some(EventTap { port, source })
}

pub fn pump(runtime: &mut Runtime, bus: &Bus, seconds: f64) {
    let mut ctx = TapContext {
        runtime: runtime as *mut Runtime,
        bus: bus as *const Bus,
    };
    CONTEXT.with(|cell| cell.set(&mut ctx));
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, 0);
    }
    CONTEXT.with(|cell| cell.set(ptr::null_mut()));
}

fn event_mask() -> u64 {
    let types = [1_u32, 2, 5, 6, 10, 11, 12, 22, 29];
    types.iter().fold(0_u64, |mask, kind| mask | (1_u64 << kind))
}

extern "C" fn tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    _user: *mut c_void,
) -> *mut c_void {
    if event_type == TAP_DISABLED_BY_TIMEOUT || event_type == TAP_DISABLED_BY_USER {
        let port = TAP_PORT.with(|cell| cell.get());
        if !port.is_null() {
            unsafe { CGEventTapEnable(port, 1) };
        }
        return event;
    }
    let ctx_ptr = CONTEXT.with(|cell| cell.get());
    if ctx_ptr.is_null() {
        return event;
    }
    let ctx = unsafe { &mut *ctx_ptr };
    let runtime = unsafe { &mut *ctx.runtime };
    let bus = unsafe { &*ctx.bus };
    if event_type == 29 {
        swipe::observe(runtime, event_type, event);
        return event;
    }
    if matches!(event_type, 1 | 2 | 5 | 6) {
        let point = unsafe { CGEventGetLocation(event) };
        runtime.note_pointer(event_type, point.x, point.y);
        return event;
    }
    if event_type == FLAGS_CHANGED {
        emit_hold(runtime, bus, event);
        return event;
    }
    if event_type != KEY_DOWN {
        return event;
    }
    let keycode = unsafe { CGEventGetIntegerValueField(event, KEYCODE_FIELD) } as u16;
    let flags = unsafe { CGEventGetFlags(event) };
    let binding = Binding {
        keycode,
        ctrl: flags & FLAG_CONTROL != 0,
        alt: flags & FLAG_ALTERNATE != 0,
        shift: flags & FLAG_SHIFT != 0,
        cmd: flags & FLAG_COMMAND != 0,
    };
    let Some(resolved) = runtime.binding_for(binding) else {
        return event;
    };
    let args = match resolved.n {
        Some(n) => serde_json::json!({ "n": n }),
        None => serde_json::json!({}),
    };
    let _ = command::run(runtime, bus, &resolved.action, &args);
    ptr::null_mut()
}

fn emit_hold(runtime: &Runtime, bus: &Bus, event: *mut c_void) {
    let flags = unsafe { CGEventGetFlags(event) };
    let held = flags & FLAG_CONTROL != 0 && flags & FLAG_ALTERNATE != 0;
    let Some(display) = runtime.primary_display() else {
        return;
    };
    bus.emit(
        "indicator",
        serde_json::json!({
            "display": display,
            "show": held,
            "hold": held,
        }),
    );
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events: u64,
        callback: extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
        user: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: u8);
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
    fn CGEventGetFlags(event: *mut c_void) -> u64;
    fn CGEventGetLocation(event: *mut c_void) -> CgPoint;
}

#[repr(C)]
struct CgPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    static kCFRunLoopCommonModes: *const c_void;
    fn CFMachPortCreateRunLoopSource(allocator: *const c_void, port: *mut c_void, order: c_uint) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    fn CFRunLoopWakeUp(run_loop: *mut c_void);
}
