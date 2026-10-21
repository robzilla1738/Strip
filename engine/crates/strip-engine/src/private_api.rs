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
/// Strips stay keyed by display UUID; this id is recorded on the session and
/// refreshed on wake. The missing-SkyLight path uses [`fallback_space_key`].
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
}
