//! Hot reload for the config file. The watcher is held for the process lifetime.
//!
//! The parent directory is watched so an atomic rename is visible. A run-loop
//! source is signaled from the callback; `CFRunLoopWakeUp` alone is lost when
//! it arrives before the loop waits, but a signaled source still fires.

use std::os::raw::c_void;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

static DIRTY: AtomicBool = AtomicBool::new(false);
static SOURCE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

pub struct ConfigWatch {
    _watcher: RecommendedWatcher,
}

pub fn watch_config(path: &Path) -> Result<ConfigWatch, String> {
    let target = path.to_path_buf();
    let stamp = std::sync::Arc::new(Mutex::new(stamp_of(&target)));
    let mut watcher = notify::recommended_watcher({
        let stamp = std::sync::Arc::clone(&stamp);
        move |result: Result<notify::Event, notify::Error>| {
            if result.is_err() {
                return;
            }
            let next = stamp_of(&target);
            let mut guard = stamp.lock().unwrap_or_else(|err| err.into_inner());
            if *guard == next {
                return;
            }
            *guard = next;
            drop(guard);
            poke_reload();
        }
    })
    .map_err(|err| err.to_string())?;
    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    watcher.watch(dir, RecursiveMode::NonRecursive).map_err(|err| err.to_string())?;
    Ok(ConfigWatch { _watcher: watcher })
}

pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::SeqCst)
}

fn poke_reload() {
    DIRTY.store(true, Ordering::SeqCst);
    let source = SOURCE.load(Ordering::SeqCst);
    if source.is_null() {
        crate::hotkeys::wake_main();
        return;
    }
    unsafe {
        CFRunLoopSourceSignal(source);
        CFRunLoopWakeUp(CFRunLoopGetMain());
    }
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
        CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
        SOURCE.store(source, Ordering::SeqCst);
    }
}

extern "C" fn perform_reload(_info: *mut c_void) {
    unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
}

fn stamp_of(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
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
    static kCFRunLoopCommonModes: *const c_void;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopSourceCreate(allocator: *const c_void, order: isize, context: *mut CFRunLoopSourceContext) -> *mut c_void;
    fn CFRunLoopSourceSignal(source: *mut c_void);
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRunLoopWakeUp(run_loop: *mut c_void);
}
