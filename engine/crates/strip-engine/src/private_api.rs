//! Every private macOS symbol used by the engine, loaded with `dlsym` and checked at startup.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::Mutex;

const RTLD_LAZY: c_int = 1;

const SKYLIGHT: &str = "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
const HISERVICES: &str = "/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/Versions/A/HIServices";

const SKYLIGHT_SYMBOLS: &[&str] = &[
    "SLSMainConnectionID",
    "SLSGetActiveSpace",
    "SLSCopyManagedDisplaySpaces",
    "SLSManagedDisplayGetCurrentSpace",
    "SLSSpaceGetType",
    "SLSCopySpacesForWindows",
    "SLSCopyManagedDisplayForWindow",
    "SLSRegisterConnectionNotifyProc",
];

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

struct Image(*mut c_void);

// The handle stays open for the process lifetime and is only used from `dlsym`.
unsafe impl Send for Image {}

pub trait SymbolSource {
    fn has(&self, image: &str, symbol: &str) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateApi {
    pub skylight: bool,
    pub ax_get_window: bool,
    pub slps: bool,
}

impl PrivateApi {
    pub fn live() -> Self {
        probe(&LiveSymbols)
    }
}

pub struct LiveSymbols;

impl SymbolSource for LiveSymbols {
    fn has(&self, image: &str, symbol: &str) -> bool {
        live_symbol(image, symbol).is_some()
    }
}

pub fn probe<S: SymbolSource + ?Sized>(source: &S) -> PrivateApi {
    let skylight = SKYLIGHT_SYMBOLS.iter().all(|symbol| source.has(SKYLIGHT, symbol));
    let ax_get_window = source.has(HISERVICES, "_AXUIElementGetWindow");
    let slps = source.has(SKYLIGHT, "_SLPSSetFrontProcessWithOptions")
        && source.has(SKYLIGHT, "SLPSPostEventRecordTo");
    PrivateApi {
        skylight,
        ax_get_window,
        slps,
    }
}

/// Active Space from SkyLight. `1` when the symbol is missing or returns 0.
pub fn current_space() -> u64 {
    let Some(main) = live_symbol_ptr(SKYLIGHT, "SLSMainConnectionID") else {
        return 1;
    };
    let Some(get) = live_symbol_ptr(SKYLIGHT, "SLSGetActiveSpace") else {
        return 1;
    };
    type Main = unsafe extern "C" fn() -> i32;
    type Get = unsafe extern "C" fn(i32) -> u64;
    let id = unsafe {
        let main: Main = std::mem::transmute(main);
        let get: Get = std::mem::transmute(get);
        get(main())
    };
    if id == 0 { 1 } else { id }
}

/// Space component of a strip key.
/// On-screen windows belong to the active Space. An off-screen window belongs
/// to the Space SkyLight reports. Without SkyLight every window shares `active`
/// (`0` until a space switch, then [`fallback_space_key`]).
pub fn assign_strip_space(skylight: bool, active: u64, window_space: Option<u64>, on_screen: bool) -> u64 {
    if !skylight || on_screen {
        return active;
    }
    window_space.unwrap_or(active)
}

/// Space id to activate after `activeSpaceDidChange`. SkyLight uses the live
/// id. The fallback hashes the on-screen window ids.
pub fn space_key_after_switch(skylight: bool, live_space: u64, on_screen_ids: &[u64]) -> u64 {
    if skylight {
        live_space.max(1)
    } else {
        fallback_space_key(on_screen_ids)
    }
}

/// FNV-1a 64 of the sorted on-screen window ids. Used when SkyLight is missing.
pub fn fallback_space_key(window_ids: &[u64]) -> u64 {
    let mut ids = window_ids.to_vec();
    ids.sort_unstable();
    let mut hash = 0xcbf29ce484222325;
    for id in ids {
        for byte in id.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

pub fn live_symbol_ptr(image: &str, symbol: &str) -> Option<*mut c_void> {
    live_symbol(image, symbol)
}

fn live_symbol(image: &str, symbol: &str) -> Option<*mut c_void> {
    let handle = open_image(image)?;
    let name = CString::new(symbol).ok()?;
    unsafe {
        let _ = dlerror();
        let found = dlsym(handle, name.as_ptr());
        if found.is_null() {
            None
        } else {
            Some(found)
        }
    }
}

#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

const CPS_USER_GENERATED: u32 = 0x200;

/// `_SLPSSetFrontProcessWithOptions` then `SLPSPostEventRecordTo`.
/// Returns false when the process or the symbols are missing.
pub fn slps_raise(pid: i32, window: u32) -> bool {
    if pid <= 0 {
        return false;
    }
    let Some(set_front) = live_symbol_ptr(SKYLIGHT, "_SLPSSetFrontProcessWithOptions") else {
        return false;
    };
    let Some(post) = live_symbol_ptr(SKYLIGHT, "SLPSPostEventRecordTo") else {
        return false;
    };
    let mut psn = ProcessSerialNumber { high: 0, low: 0 };
    if unsafe { GetProcessForPID(pid, &mut psn) } != 0 {
        return false;
    }
    type SetFront = unsafe extern "C" fn(*const ProcessSerialNumber, u32, u32) -> i32;
    type Post = unsafe extern "C" fn(*const ProcessSerialNumber, *const u8) -> i32;
    let set_front: SetFront = unsafe { std::mem::transmute(set_front) };
    let post: Post = unsafe { std::mem::transmute(post) };
    let front_err = unsafe { set_front(&psn, window, CPS_USER_GENERATED) };
    let mut event = [0u8; 0xf8];
    event[0x04] = 0xf8;
    event[0x3a] = 0x10;
    event[0x3c..0x40].copy_from_slice(&window.to_ne_bytes());
    event[0x08] = 0x01;
    let first = unsafe { post(&psn, event.as_ptr()) };
    event[0x08] = 0x02;
    let second = unsafe { post(&psn, event.as_ptr()) };
    front_err == 0 || first == 0 || second == 0
}

fn connection() -> Option<i32> {
    let main = live_symbol_ptr(SKYLIGHT, "SLSMainConnectionID")?;
    type Main = unsafe extern "C" fn() -> i32;
    Some(unsafe { std::mem::transmute::<*mut c_void, Main>(main)() })
}

fn sky<T>(name: &str) -> Option<T> {
    let ptr = live_symbol_ptr(SKYLIGHT, name)?;
    Some(unsafe { std::mem::transmute_copy(&ptr) })
}

/// Space ids from `SLSCopyManagedDisplaySpaces`.
pub fn managed_space_ids() -> Vec<u64> {
    let Some(cid) = connection() else {
        return Vec::new();
    };
    let Some(copy) = sky::<unsafe extern "C" fn(i32) -> *const c_void>("SLSCopyManagedDisplaySpaces") else {
        return Vec::new();
    };
    let array = unsafe { copy(cid) };
    if array.is_null() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    unsafe {
        let displays = CFArrayGetCount(array);
        for index in 0..displays {
            let dict = CFArrayGetValueAtIndex(array, index);
            if dict.is_null() {
                continue;
            }
            let spaces = dict_get(dict, "Spaces");
            if spaces.is_null() {
                continue;
            }
            let count = CFArrayGetCount(spaces);
            for space_index in 0..count {
                let space = CFArrayGetValueAtIndex(spaces, space_index);
                if let Some(id) = number_u64(dict_get(space, "id64")) {
                    ids.push(id);
                }
            }
        }
        CFRelease(array);
    }
    ids
}

fn windows_on_space(cid: i32, space: u64) -> Vec<u64> {
    let Some(copy) = sky::<unsafe extern "C" fn(i32, u32, *const c_void, u32, *mut i64, *mut i64) -> *const c_void>(
        "SLSCopyWindowsWithOptionsAndTags",
    ) else {
        return Vec::new();
    };
    let number = cf_number(i64::try_from(space).unwrap_or(0));
    let array = cf_array(&[number]);
    let mut set_tags = 0i64;
    let mut clear_tags = 0i64;
    let list = unsafe { copy(cid, 0, array, 0x2, &mut set_tags, &mut clear_tags) };
    unsafe {
        CFRelease(number);
        CFRelease(array);
    }
    if list.is_null() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    unsafe {
        let count = CFArrayGetCount(list);
        for index in 0..count {
            if let Some(id) = number_u64(CFArrayGetValueAtIndex(list, index)) {
                ids.push(id);
            }
        }
        CFRelease(list);
    }
    ids
}

/// Windows SkyLight lists that are not in `on_screen`, with their Space id.
pub fn windows_outside(on_screen: &[u64]) -> Vec<(u64, u64)> {
    let Some(cid) = connection() else {
        return Vec::new();
    };
    let known: std::collections::HashSet<u64> = on_screen.iter().copied().collect();
    let mut found = Vec::new();
    for space in managed_space_ids() {
        for id in windows_on_space(cid, space) {
            if !known.contains(&id) {
                found.push((id, space));
            }
        }
    }
    found
}

/// First Space that contains `window`, from `SLSCopySpacesForWindows`.
pub fn space_for_window(window: u64) -> Option<u64> {
    let cid = connection()?;
    let copy = sky::<unsafe extern "C" fn(i32, i32, *const c_void) -> *const c_void>("SLSCopySpacesForWindows")?;
    let number = cf_number(i64::try_from(window).unwrap_or(0));
    let array = cf_array(&[number]);
    let spaces = unsafe { copy(cid, 0x7, array) };
    unsafe {
        CFRelease(number);
        CFRelease(array);
    }
    if spaces.is_null() {
        return None;
    }
    unsafe {
        let id = if CFArrayGetCount(spaces) > 0 {
            number_u64(CFArrayGetValueAtIndex(spaces, 0))
        } else {
            None
        };
        CFRelease(spaces);
        id
    }
}

/// `SLSSpaceGetType` 4 is a fullscreen Space.
pub fn space_is_fullscreen(space: u64) -> bool {
    let Some(cid) = connection() else {
        return false;
    };
    let Some(get) = sky::<unsafe extern "C" fn(i32, u64) -> i32>("SLSSpaceGetType") else {
        return false;
    };
    unsafe { get(cid, space) == 4 }
}

fn dict_get(dict: *const c_void, key: &str) -> *const c_void {
    if dict.is_null() {
        return std::ptr::null();
    }
    let name = match std::ffi::CString::new(key) {
        Ok(name) => name,
        Err(_) => return std::ptr::null(),
    };
    unsafe {
        let cf = CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), 0x0800_0100);
        if cf.is_null() {
            return std::ptr::null();
        }
        let value = CFDictionaryGetValue(dict, cf);
        CFRelease(cf);
        value
    }
}

fn number_u64(value: *const c_void) -> Option<u64> {
    if value.is_null() {
        return None;
    }
    unsafe {
        let mut wide = 0i64;
        if CFNumberGetValue(value, 4, (&mut wide as *mut i64).cast()) != 0 {
            return u64::try_from(wide).ok();
        }
        let mut narrow = 0i32;
        if CFNumberGetValue(value, 3, (&mut narrow as *mut i32).cast()) != 0 {
            return u64::try_from(narrow).ok();
        }
    }
    None
}

fn cf_number(value: i64) -> *const c_void {
    unsafe { CFNumberCreate(std::ptr::null(), 4, (&value as *const i64).cast()) }
}

fn cf_array(values: &[*const c_void]) -> *const c_void {
    unsafe {
        CFArrayCreate(
            std::ptr::null(),
            values.as_ptr(),
            values.len() as isize,
            &kCFTypeArrayCallBacks,
        )
    }
}

fn open_image(image: &str) -> Option<*mut c_void> {
    static HANDLES: Mutex<Option<HashMap<String, Image>>> = Mutex::new(None);
    let mut guard = HANDLES.lock().unwrap_or_else(|err| err.into_inner());
    let handles = guard.get_or_insert_with(HashMap::new);
    if let Some(handle) = handles.get(image) {
        return Some(handle.0);
    }
    let path = CString::new(image).ok()?;
    let handle = unsafe { dlopen(path.as_ptr(), RTLD_LAZY) };
    if handle.is_null() {
        return None;
    }
    handles.insert(image.to_string(), Image(handle));
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        missing: &'static str,
    }

    impl SymbolSource for Scripted {
        fn has(&self, _image: &str, symbol: &str) -> bool {
            symbol != self.missing
        }
    }

    struct Present;

    impl SymbolSource for Present {
        fn has(&self, _image: &str, _symbol: &str) -> bool {
            true
        }
    }

    #[test]
    fn missing_skylight_symbol_disables_the_flag() {
        let flags = probe(&Scripted {
            missing: "SLSSpaceGetType",
        });
        assert!(!flags.skylight);
        assert!(flags.ax_get_window);
    }

    #[test]
    fn all_symbols_present_enables_every_flag() {
        let flags = probe(&Present);
        assert!(flags.skylight);
        assert!(flags.ax_get_window);
        assert!(flags.slps);
    }

    #[test]
    fn live_probe_checks_symbols_without_panicking() {
        let flags = PrivateApi::live();
        let _ = (flags.skylight, flags.ax_get_window, flags.slps);
    }

    #[test]
    fn current_space_is_at_least_one() {
        assert!(current_space() >= 1);
    }

    #[test]
    fn fallback_space_key_ignores_order_and_changes_with_membership() {
        assert_eq!(fallback_space_key(&[3, 1, 2]), fallback_space_key(&[1, 2, 3]));
        assert_ne!(fallback_space_key(&[1, 2, 3]), fallback_space_key(&[1, 2, 4]));
        assert_eq!(fallback_space_key(&[]), fallback_space_key(&[]));
    }

    #[test]
    fn on_screen_windows_stay_on_the_active_space() {
        assert_eq!(assign_strip_space(true, 9, Some(4), true), 9);
        assert_eq!(assign_strip_space(true, 9, Some(4), false), 4);
        assert_eq!(assign_strip_space(false, 0, Some(4), false), 0);
    }

    #[test]
    fn space_switch_uses_the_fallback_hash_without_skylight() {
        let ids = [3u64, 1, 2];
        assert_eq!(space_key_after_switch(false, 9, &ids), fallback_space_key(&ids));
        assert_eq!(space_key_after_switch(true, 9, &ids), 9);
        assert_eq!(space_key_after_switch(true, 0, &ids), 1);
    }

    #[test]
    fn slps_raise_rejects_a_missing_process() {
        assert!(!slps_raise(0, 1));
    }

    #[test]
    fn other_space_query_does_not_panic() {
        let _ = windows_outside(&[]);
        let _ = space_for_window(1);
        assert!(!space_is_fullscreen(0));
    }
}

#[repr(C)]
struct CFArrayCallBacks {
    version: isize,
    retain: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void, *const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void>,
    equal: Option<unsafe extern "C" fn(*const c_void, *const c_void, *const c_void) -> u8>,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeArrayCallBacks: CFArrayCallBacks;
    fn CFArrayCreate(
        allocator: *const c_void,
        values: *const *const c_void,
        count: isize,
        callbacks: *const CFArrayCallBacks,
    ) -> *const c_void;
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberCreate(allocator: *const c_void, kind: i32, value: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: i32, out: *mut c_void) -> u8;
    fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
    fn CFRelease(value: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn GetProcessForPID(pid: i32, psn: *mut ProcessSerialNumber) -> i32;
}
