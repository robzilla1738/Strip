//! Hot reload for the config file. The watcher is held for the process lifetime.
//!
//! The parent directory is watched so an atomic rename is visible. A run-loop
//! source is signaled from the callback; `CFRunLoopWakeUp` alone is lost when
//! it arrives before the loop waits, but a signaled source still fires.
//!
//! `notify` is CC0-1.0, which is outside the license allow-list, so this uses
//! FSEvents through `fsevent-sys` (MIT).

use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::runloop::kCFRunLoopCommonModes as RUN_LOOP_COMMON;
use core_foundation::runloop::CFRunLoop;
use core_foundation::string::CFString;
use fsevent_sys::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer,
    kFSEventStreamEventIdSinceNow, FSEventStreamContext, FSEventStreamCreate,
    FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamInvalidate, FSEventStreamRef,
    FSEventStreamRelease, FSEventStreamScheduleWithRunLoop, FSEventStreamStart, FSEventStreamStop,
};

static DIRTY: AtomicBool = AtomicBool::new(false);
static SOURCE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

struct WatchState {
    target: PathBuf,
    stamp: Mutex<Option<(u128, u64)>>,
}

pub struct ConfigWatch {
    stream: FSEventStreamRef,
    state: *mut WatchState,
}

pub fn watch_config(path: &Path) -> Result<ConfigWatch, String> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let state = Box::into_raw(Box::new(WatchState {
        target: path.to_path_buf(),
        stamp: Mutex::new(stamp_of(path)),
    }));
    let watched = CFString::new(&dir.to_string_lossy());
    let paths = CFArray::from_CFTypes(&[watched]);
    let context = FSEventStreamContext {
        version: 0,
        info: state.cast(),
        retain: None,
        release: None,
        copy_description: None,
    };
    unsafe {
        let stream = FSEventStreamCreate(
            ptr::null_mut(),
            on_fs_event,
            &context,
            paths.as_concrete_TypeRef().cast::<c_void>().cast_mut(),
            kFSEventStreamEventIdSinceNow,
            0.0,
            kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagFileEvents,
        );
        if stream.is_null() {
            drop(Box::from_raw(state));
            return Err("FSEventStreamCreate failed".to_string());
        }
        let run_loop = CFRunLoop::get_main();
        FSEventStreamScheduleWithRunLoop(
            stream,
            run_loop.as_concrete_TypeRef().cast(),
            RUN_LOOP_COMMON.cast::<c_void>().cast_mut(),
        );
        if FSEventStreamStart(stream) == 0 {
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            drop(Box::from_raw(state));
            return Err("FSEventStreamStart failed".to_string());
        }
        Ok(ConfigWatch { stream, state })
    }
}

impl Drop for ConfigWatch {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                FSEventStreamStop(self.stream);
                FSEventStreamInvalidate(self.stream);
                FSEventStreamRelease(self.stream);
            }
            if !self.state.is_null() {
                drop(Box::from_raw(self.state));
            }
        }
    }
}

extern "C" fn on_fs_event(
    _stream: FSEventStreamRef,
    info: *mut c_void,
    _num_events: usize,
    _event_paths: *mut c_void,
    _flags: *const FSEventStreamEventFlags,
    _ids: *const FSEventStreamEventId,
) {
    if info.is_null() {
        return;
    }
    let state = unsafe { &*(info as *const WatchState) };
    let next = stamp_of(&state.target);
    let mut guard = state.stamp.lock().unwrap_or_else(|err| err.into_inner());
    if *guard == next {
        return;
    }
    *guard = next;
    drop(guard);
    poke_reload();
}

pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::SeqCst)
}

/// Signals the main-loop source so a wakeup that lands before `CFRunLoopRunInMode`
/// begins waiting still causes it to return immediately.
pub fn signal_main_source() {
    let source = SOURCE.load(Ordering::SeqCst);
    if !source.is_null() {
        unsafe {
            CFRunLoopSourceSignal(source);
        }
    }
}

fn poke_reload() {
    DIRTY.store(true, Ordering::SeqCst);
    signal_main_source();
    crate::hotkeys::wake_main();
}

pub fn install_reload_source() {
    unsafe {
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: ptr::null_mut(),
            retain: None,
            release: None,
            copy_description: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(perform_reload),
        };
        let source = CFRunLoopSourceCreate(ptr::null(), 0, &mut context);
        if source.is_null() {
            return;
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), source, RUN_LOOP_COMMON.cast());
        SOURCE.store(source, Ordering::SeqCst);
    }
}

extern "C" fn perform_reload(_info: *mut c_void) {
    unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
}

fn stamp_of(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((modified, meta.len()))
}

#[repr(C)]
struct CFRunLoopSourceContext {
    version: isize,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> *const c_void>,
    equal: Option<extern "C" fn(*const c_void, *const c_void) -> u8>,
    hash: Option<extern "C" fn(*const c_void) -> usize>,
    schedule: Option<extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    cancel: Option<extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    perform: Option<extern "C" fn(*mut c_void)>,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopSourceCreate(
        allocator: *const c_void,
        order: isize,
        context: *mut CFRunLoopSourceContext,
    ) -> *mut c_void;
    fn CFRunLoopSourceSignal(source: *mut c_void);
    fn CFRunLoopStop(run_loop: *mut c_void);
}
