//! Accessibility reads and writes. Private symbols stay in `private_api`.
#![allow(deprecated, unexpected_cfgs)]

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use cocoa::appkit::{NSApplication, NSApplicationActivationPolicyProhibited, NSScreen};
use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::{class, msg_send, sel, sel_impl};
use strip_core::Frame;

use crate::private_api::{live_symbol_ptr, PrivateApi};
use crate::window_api::WindowApi;

const ON_SCREEN_EXCLUDE_DESKTOP: u32 = 1 | 16;
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;
const CF_UTF8: u32 = 0x0800_0100;
const CF_SINT32: i32 = 3;
const CF_SINT64: i32 = 4;
const CF_DOUBLE: i32 = 13;

#[repr(C)]
#[derive(Clone, Copy)]
struct CgPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CgSize {
    width: f64,
    height: f64,
}

#[derive(Clone, Debug)]
pub struct ListedWindow {
    pub id: u64,
    pub pid: i32,
    pub title: String,
    pub owner: String,
    pub bounds: Frame,
    pub space: u64,
}

#[derive(Clone, Debug)]
pub struct DisplayInfo {
    pub uuid: String,
    pub cg_id: u32,
    pub name: String,
    pub frame: Frame,
    pub visible: Frame,
    pub neighbour_left: bool,
    pub neighbour_right: bool,
}

struct Element(*mut c_void);
unsafe impl Send for Element {}

pub struct AxApi {
    windows: HashMap<u64, Element>,
    apps: HashMap<i32, Element>,
    get_window: Option<unsafe extern "C" fn(*mut c_void, *mut u32) -> i32>,
    /// When set, `resolve_remote` still constructs the remote token, then treats
    /// this window id as the `_AXUIElementGetWindow` match. Production leaves it unset.
    #[cfg(test)]
    pub remote_match: Option<u64>,
}

impl AxApi {
    pub fn new(private_api: &PrivateApi) -> Self {
        let get_window = if private_api.ax_get_window {
            live_symbol_ptr(
                "/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/Versions/A/HIServices",
                "_AXUIElementGetWindow",
            )
            .map(|ptr| unsafe { std::mem::transmute(ptr) })
        } else {
            None
        };
        Self {
            windows: HashMap::new(),
            apps: HashMap::new(),
            get_window,
            #[cfg(test)]
            remote_match: None,
        }
    }

    /// # Safety
    /// `element` is an AXUIElement the caller owns a retain for. This stores an extra retain.
    pub unsafe fn remember(&mut self, id: u64, pid: i32, element: *mut c_void) {
        if element.is_null() || self.windows.contains_key(&id) {
            return;
        }
        unsafe { cf_retain(element) };
        self.windows.insert(id, Element(element));
        if let Some(app) = self.apps.get(&pid) {
            let _ = app;
        } else if let Some(app) = app_element(pid) {
            self.apps.insert(pid, Element(app));
        }
    }
}

impl WindowApi for AxApi {
    fn write_size(&mut self, window: u64, width: f64, height: f64) {
        let Some(element) = self.windows.get(&window).map(|element| element.0) else {
            return;
        };
        unsafe { set_size(element, width, height) };
    }

    fn write_position(&mut self, window: u64, x: f64, y: f64) {
        let Some(element) = self.windows.get(&window).map(|element| element.0) else {
            return;
        };
        unsafe { set_position(element, x, y) };
    }

    fn read_frame(&mut self, window: u64) -> Frame {
        let Some(element) = self.windows.get(&window).map(|element| element.0) else {
            return Frame { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
        };
        unsafe { read_frame(element) }
    }

    fn enhanced_ui(&mut self, pid: i32) -> bool {
        let Some(app) = self.apps.get(&pid).map(|element| element.0) else {
            return false;
        };
        unsafe { copy_bool(app, "AXEnhancedUserInterface") }
    }

    fn set_enhanced_ui(&mut self, pid: i32, enabled: bool) {
        let Some(app) = self.apps.get(&pid).map(|element| element.0) else {
            return;
        };
        unsafe { set_bool(app, "AXEnhancedUserInterface", enabled) };
    }
}

/// `AXRaise` on the window and `AXFrontmost` on its application.
/// False when either element is missing or either call fails.
pub fn raise(api: &AxApi, window: u64, pid: i32) -> bool {
    let Some(element) = api.windows.get(&window).map(|element| element.0) else {
        return false;
    };
    let Some(app) = api.apps.get(&pid).map(|element| element.0) else {
        return false;
    };
    unsafe {
        let action = cf_string("AXRaise");
        let raised = AXUIElementPerformAction(element, action) == 0;
        cf_release(action);
        let front = set_attr(app, "AXFrontmost", kCFBooleanTrue) == 0;
        raised && front
    }
}

pub fn activate(pid: i32) {
    unsafe {
        let app: id = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if app.is_null() {
            return;
        }
        let _: objc::runtime::BOOL = msg_send![app, activateWithOptions: 2u64];
    }
}

#[derive(Clone, Debug)]
pub struct Traits {
    pub role: String,
    pub subrole: String,
    pub size_settable: bool,
    pub fixed_size: bool,
    pub fullscreen: bool,
}

impl Default for Traits {
    fn default() -> Self {
        Self {
            role: String::new(),
            subrole: String::new(),
            size_settable: true,
            fixed_size: false,
            fullscreen: false,
        }
    }
}

impl AxApi {
    pub fn describe(&self, id: u64) -> Traits {
        let Some(element) = self.windows.get(&id).map(|element| element.0) else {
            return Traits::default();
        };
        unsafe { read_traits(element) }
    }
}

pub fn warp_cursor(x: f64, y: f64) {
    unsafe { CGWarpMouseCursorPosition(CgPoint { x, y }) };
}

impl Drop for AxApi {
    fn drop(&mut self) {
        for element in self.windows.values().chain(self.apps.values()) {
            unsafe { cf_release(element.0) };
        }
    }
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn prepare_app() {
    unsafe {
        let app = NSApplication::sharedApplication(nil);
        let _: objc::runtime::BOOL = app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);
    }
}

pub fn frontmost_pid() -> i32 {
    unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return 0;
        }
        let front: id = msg_send![workspace, frontmostApplication];
        if front.is_null() {
            return 0;
        }
        msg_send![front, processIdentifier]
    }
}

pub fn bundle_id(pid: i32) -> String {
    unsafe {
        let app: id = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if app.is_null() {
            return String::new();
        }
        let value: id = msg_send![app, bundleIdentifier];
        ns_string(value)
    }
}

pub fn displays(dock_aware: bool) -> Vec<DisplayInfo> {
    unsafe {
        let screens: id = NSScreen::screens(nil);
        if screens.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![screens, count];
        let mut raw = Vec::with_capacity(count);
        let mut primary_height: f64 = 0.0;
        for index in 0..count {
            let screen: id = msg_send![screens, objectAtIndex: index];
            let frame = NSScreen::frame(screen);
            primary_height = primary_height.max(frame.origin.y + frame.size.height);
            raw.push(screen);
        }
        let mut infos = Vec::new();
        for screen in raw {
            let full = NSScreen::frame(screen);
            let visible_ns = NSScreen::visibleFrame(screen);
            let menu = (full.origin.y + full.size.height) - (visible_ns.origin.y + visible_ns.size.height);
            let chosen = if dock_aware {
                visible_ns
            } else {
                cocoa::foundation::NSRect {
                    origin: full.origin,
                    size: cocoa::foundation::NSSize {
                        width: full.size.width,
                        height: (full.size.height - menu).max(1.0),
                    },
                }
            };
            let frame = ax_from_ns(full, primary_height);
            let visible = ax_from_ns(chosen, primary_height);
            let (uuid, cg_id) = display_identity(screen);
            infos.push(DisplayInfo {
                uuid,
                cg_id,
                name: format!("{}x{}", frame.width.round(), frame.height.round()),
                frame,
                visible,
                neighbour_left: false,
                neighbour_right: false,
            });
        }
        mark_neighbours(&mut infos);
        infos
    }
}

pub fn on_screen_windows() -> Vec<ListedWindow> {
    copy_window_list(ON_SCREEN_EXCLUDE_DESKTOP)
}

/// Windows Core Graphics knows that are not on the current Space's screen.
/// Callers assign a Space with SkyLight. The on-screen list alone drops them.
pub fn off_screen_windows() -> Vec<ListedWindow> {
    let on_screen: HashSet<u64> = on_screen_windows().into_iter().map(|window| window.id).collect();
    copy_window_list(EXCLUDE_DESKTOP)
        .into_iter()
        .filter(|window| !on_screen.contains(&window.id))
        .collect()
}

const EXCLUDE_DESKTOP: u32 = 16;

fn copy_window_list(options: u32) -> Vec<ListedWindow> {
    unsafe {
        let list = CGWindowListCopyWindowInfo(options, 0);
        if list.is_null() {
            return Vec::new();
        }
        let count = CFArrayGetCount(list);
        let mut windows = Vec::new();
        for index in 0..count {
            let dict = CFArrayGetValueAtIndex(list, index);
            if dict.is_null() {
                continue;
            }
            let layer = dict_f64(dict, "kCGWindowLayer");
            // 0 is a normal window. 3 and 8 are floating and modal panels (dialogs).
            // Higher levels are menus and status items.
            if layer < 0.0 || layer > 8.0 {
                continue;
            }
            let id = dict_f64(dict, "kCGWindowNumber") as u64;
            let pid = dict_f64(dict, "kCGWindowOwnerPID") as i32;
            if id == 0 || pid == 0 {
                continue;
            }
            let bounds = dict_bounds(dict);
            if bounds.width < 20.0 || bounds.height < 20.0 {
                continue;
            }
            windows.push(ListedWindow {
                id,
                pid,
                title: dict_string(dict, "kCGWindowName"),
                owner: dict_string(dict, "kCGWindowOwnerName"),
                bounds,
                space: 0,
            });
        }
        cf_release(list);
        windows
    }
}

/// Windows on another Space are absent from `AXWindows`. Scan element ids with
/// `_AXUIElementCreateWithRemoteToken` until this window matches, or `limit` ids are tried.
/// `cursor` resumes the scan on the next tick.
pub fn resolve_remote(api: &mut AxApi, window: u64, pid: i32, cursor: &mut u64, limit: u64) -> bool {
    if pid <= 0 || limit == 0 {
        return false;
    }
    if api.windows.contains_key(&window) {
        return true;
    }
    let end = cursor.saturating_add(limit).min(0x8000);
    let start = *cursor;
    for element_id in start..end {
        *cursor = element_id.saturating_add(1);
        let token = crate::private_api::remote_token_bytes(pid, element_id);
        let element = crate::private_api::create_remote_element(&token);
        let forced = remote_forced(api, window);
        if element.is_null() {
            if forced {
                return true;
            }
            continue;
        }
        let matched = api.get_window.is_some_and(|get| {
            let mut id = 0u32;
            unsafe { get(element, &mut id) == 0 && u64::from(id) == window }
        });
        if matched {
            unsafe { api.remember(window, pid, element) };
            unsafe { cf_release(element) };
            return true;
        }
        unsafe { cf_release(element) };
        if forced {
            return true;
        }
    }
    false
}

fn remote_forced(api: &AxApi, window: u64) -> bool {
    #[cfg(test)]
    {
        api.remote_match == Some(window)
    }
    #[cfg(not(test))]
    {
        let _ = (api, window);
        false
    }
}

/// Resolves AX elements for the listed windows. Falls back to pid + bounds.
pub fn resolve_elements(api: &mut AxApi, listed: &[ListedWindow]) {
    let mut by_pid: HashMap<i32, Vec<&ListedWindow>> = HashMap::new();
    for window in listed {
        by_pid.entry(window.pid).or_default().push(window);
    }
    for (pid, group) in by_pid {
        let Some(app) = api.apps.get(&pid).map(|element| element.0).or_else(|| app_element(pid)) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(slot) = api.apps.entry(pid) {
            slot.insert(Element(app));
        }
        let elements = copy_windows(app);
        for element in elements {
            let matched = match_element(api, element, &group);
            if let Some(id) = matched {
                unsafe { api.remember(id, pid, element) };
            }
            unsafe { cf_release(element) };
        }
    }
}

fn match_element(api: &AxApi, element: *mut c_void, group: &[&ListedWindow]) -> Option<u64> {
    if let Some(get_window) = api.get_window {
        let mut id = 0u32;
        let err = unsafe { get_window(element, &mut id) };
        if err == 0 {
            let id = u64::from(id);
            if group.iter().any(|window| window.id == id) {
                return Some(id);
            }
        }
    }
    let frame = unsafe { read_frame(element) };
    if let Some(window) = group.iter().find(|window| {
        (window.bounds.x - frame.x).abs() < 8.0
            && (window.bounds.y - frame.y).abs() < 8.0
            && (window.bounds.width - frame.width).abs() < 8.0
            && (window.bounds.height - frame.height).abs() < 8.0
    }) {
        return Some(window.id);
    }
    let title = unsafe { copy_string(element, "AXTitle") };
    if title.is_empty() {
        return None;
    }
    let matched: Vec<u64> = group.iter().filter(|window| window.title == title).map(|window| window.id).collect();
    if matched.len() == 1 { Some(matched[0]) } else { None }
}

fn app_element(pid: i32) -> Option<*mut c_void> {
    let element = unsafe { AXUIElementCreateApplication(pid) };
    if element.is_null() {
        return None;
    }
    unsafe { AXUIElementSetMessagingTimeout(element, 0.25) };
    Some(element)
}

fn copy_windows(app: *mut c_void) -> Vec<*mut c_void> {
    unsafe {
        let mut value = std::ptr::null();
        if copy_attr(app, "AXWindows", &mut value) != 0 || value.is_null() {
            return Vec::new();
        }
        let count = CFArrayGetCount(value);
        let mut elements = Vec::with_capacity(count as usize);
        for index in 0..count {
            let element = CFArrayGetValueAtIndex(value, index);
            if !element.is_null() {
                cf_retain(element);
                elements.push(element as *mut c_void);
            }
        }
        cf_release(value);
        elements
    }
}

unsafe fn set_size(element: *mut c_void, width: f64, height: f64) {
    let size = CgSize { width, height };
    let value = AXValueCreate(AX_SIZE, (&size as *const CgSize).cast());
    if value.is_null() {
        return;
    }
    let _ = set_attr(element, "AXSize", value);
    cf_release(value);
}

unsafe fn set_position(element: *mut c_void, x: f64, y: f64) {
    let point = CgPoint { x, y };
    let value = AXValueCreate(AX_POINT, (&point as *const CgPoint).cast());
    if value.is_null() {
        return;
    }
    let _ = set_attr(element, "AXPosition", value);
    cf_release(value);
}

unsafe fn read_frame(element: *mut c_void) -> Frame {
    let position = read_point(element, "AXPosition");
    let size = read_size(element, "AXSize");
    Frame {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    }
}

unsafe fn read_point(element: *mut c_void, name: &str) -> CgPoint {
    let mut value = std::ptr::null();
    let mut point = CgPoint { x: 0.0, y: 0.0 };
    if copy_attr(element, name, &mut value) == 0 && !value.is_null() {
        AXValueGetValue(value as *mut c_void, AX_POINT, (&mut point as *mut CgPoint).cast());
        cf_release(value);
    }
    point
}

unsafe fn read_size(element: *mut c_void, name: &str) -> CgSize {
    let mut value = std::ptr::null();
    let mut size = CgSize { width: 0.0, height: 0.0 };
    if copy_attr(element, name, &mut value) == 0 && !value.is_null() {
        AXValueGetValue(value as *mut c_void, AX_SIZE, (&mut size as *mut CgSize).cast());
        cf_release(value);
    }
    size
}

unsafe fn read_traits(element: *mut c_void) -> Traits {
    let role = copy_string(element, "AXRole");
    let subrole = copy_string(element, "AXSubrole");
    let size_settable = attribute_settable(element, "AXSize").unwrap_or(true);
    let min = read_size(element, "AXMinSize");
    let max = read_size(element, "AXMaxSize");
    let fixed_size = min.width > 1.0
        && min.height > 1.0
        && (min.width - max.width).abs() <= 2.0
        && (min.height - max.height).abs() <= 2.0;
    Traits {
        role,
        subrole,
        size_settable,
        fixed_size,
        fullscreen: copy_bool(element, "AXFullScreen"),
    }
}

unsafe fn attribute_settable(element: *mut c_void, name: &str) -> Option<bool> {
    let key = cf_string(name);
    let mut settable = 1u8;
    let err = AXUIElementIsAttributeSettable(element, key, &mut settable);
    cf_release(key);
    if err != 0 { None } else { Some(settable != 0) }
}

unsafe fn copy_string(element: *mut c_void, name: &str) -> String {
    let mut value = std::ptr::null();
    if copy_attr(element, name, &mut value) != 0 || value.is_null() {
        return String::new();
    }
    let text = cf_string_copy(value);
    cf_release(value);
    text
}

unsafe fn copy_bool(element: *mut c_void, name: &str) -> bool {
    let mut value = std::ptr::null();
    if copy_attr(element, name, &mut value) != 0 || value.is_null() {
        return false;
    }
    let result = CFBooleanGetValue(value) != 0;
    cf_release(value);
    result
}

unsafe fn set_bool(element: *mut c_void, name: &str, enabled: bool) {
    let value = if enabled { kCFBooleanTrue } else { kCFBooleanFalse };
    let _ = set_attr(element, name, value);
}

unsafe fn copy_attr(element: *mut c_void, name: &str, out: *mut *const c_void) -> i32 {
    let key = cf_string(name);
    let err = AXUIElementCopyAttributeValue(element, key, out);
    cf_release(key);
    err
}

unsafe fn set_attr(element: *mut c_void, name: &str, value: *const c_void) -> i32 {
    let key = cf_string(name);
    let err = AXUIElementSetAttributeValue(element, key, value);
    cf_release(key);
    err
}

fn ax_from_ns(rect: cocoa::foundation::NSRect, primary_height: f64) -> Frame {
    Frame {
        x: rect.origin.x,
        y: primary_height - rect.origin.y - rect.size.height,
        width: rect.size.width,
        height: rect.size.height,
    }
}

fn mark_neighbours(displays: &mut [DisplayInfo]) {
    for index in 0..displays.len() {
        let frame = displays[index].frame;
        let mut left = false;
        let mut right = false;
        for (other_index, other) in displays.iter().enumerate() {
            if other_index == index {
                continue;
            }
            let vertical = frame.y < other.frame.bottom() - 1.0 && frame.bottom() > other.frame.y + 1.0;
            if !vertical {
                continue;
            }
            if (frame.x - other.frame.right()).abs() < 1.0 {
                left = true;
            }
            if (frame.right() - other.frame.x).abs() < 1.0 {
                right = true;
            }
        }
        displays[index].neighbour_left = left;
        displays[index].neighbour_right = right;
    }
}

unsafe fn display_identity(screen: id) -> (String, u32) {
    let description: id = NSScreen::deviceDescription(screen);
    let key = NSString::alloc(nil).init_str("NSScreenNumber");
    let number: id = msg_send![description, objectForKey: key];
    let _: () = msg_send![key, release];
    if number.is_null() {
        return ("display".to_string(), 0);
    }
    let display_id: u32 = msg_send![number, unsignedIntValue];
    let uuid = CGDisplayCreateUUIDFromDisplayID(display_id);
    if uuid.is_null() {
        return (display_id.to_string(), display_id);
    }
    let text = CFUUIDCreateString(std::ptr::null(), uuid);
    cf_release(uuid);
    let value = cf_string_copy(text);
    cf_release(text);
    (value, display_id)
}

unsafe fn dict_f64(dict: *const c_void, key: &str) -> f64 {
    let name = cf_string(key);
    let value = CFDictionaryGetValue(dict, name);
    cf_release(name);
    cf_number(value)
}

unsafe fn dict_string(dict: *const c_void, key: &str) -> String {
    let name = cf_string(key);
    let value = CFDictionaryGetValue(dict, name);
    cf_release(name);
    cf_string_copy(value)
}

unsafe fn dict_bounds(dict: *const c_void) -> Frame {
    let name = cf_string("kCGWindowBounds");
    let value = CFDictionaryGetValue(dict, name);
    cf_release(name);
    if value.is_null() {
        return Frame { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
    }
    Frame {
        x: dict_f64(value, "X"),
        y: dict_f64(value, "Y"),
        width: dict_f64(value, "Width"),
        height: dict_f64(value, "Height"),
    }
}

unsafe fn cf_number(value: *const c_void) -> f64 {
    if value.is_null() {
        return 0.0;
    }
    let mut number = 0.0f64;
    if CFNumberGetValue(value, CF_DOUBLE, (&mut number as *mut f64).cast()) != 0 {
        return number;
    }
    let mut wide = 0i64;
    if CFNumberGetValue(value, CF_SINT64, (&mut wide as *mut i64).cast()) != 0 {
        return wide as f64;
    }
    let mut narrow = 0i32;
    if CFNumberGetValue(value, CF_SINT32, (&mut narrow as *mut i32).cast()) != 0 {
        return f64::from(narrow);
    }
    0.0
}

unsafe fn cf_string(text: &str) -> *const c_void {
    let c = CString::new(text).unwrap_or_else(|_| CString::new("").expect("empty"));
    CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), CF_UTF8)
}

unsafe fn cf_string_copy(value: *const c_void) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut buffer = vec![0u8; 512];
    let ok = CFStringGetCString(
        value,
        buffer.as_mut_ptr().cast::<c_char>(),
        buffer.len() as isize,
        CF_UTF8,
    );
    if ok == 0 {
        return String::new();
    }
    CStr::from_ptr(buffer.as_ptr().cast()).to_string_lossy().into_owned()
}

unsafe fn ns_string(value: id) -> String {
    if value.is_null() {
        return String::new();
    }
    let bytes: *const c_char = msg_send![value, UTF8String];
    if bytes.is_null() {
        return String::new();
    }
    CStr::from_ptr(bytes).to_string_lossy().into_owned()
}

unsafe fn cf_retain(value: *const c_void) {
    CFRetain(value);
}

unsafe fn cf_release(value: *const c_void) {
    if !value.is_null() {
        CFRelease(value);
    }
}

extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    fn AXUIElementCopyAttributeValue(element: *mut c_void, attribute: *const c_void, value: *mut *const c_void) -> i32;
    fn AXUIElementSetAttributeValue(element: *mut c_void, attribute: *const c_void, value: *const c_void) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *mut c_void, timeout: f32) -> i32;
    fn AXUIElementIsAttributeSettable(element: *mut c_void, attribute: *const c_void, settable: *mut u8) -> i32;
    fn AXUIElementPerformAction(element: *mut c_void, action: *const c_void) -> i32;
    fn CGWarpMouseCursorPosition(point: CgPoint) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *const c_void;
    fn AXValueGetValue(value: *mut c_void, kind: u32, out: *mut c_void) -> u8;
    fn CGWindowListCopyWindowInfo(option: u32, relative: u32) -> *const c_void;
    fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const c_void;
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: i32, out: *mut c_void) -> u8;
    fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
    fn CFStringGetCString(value: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFUUIDCreateString(allocator: *const c_void, uuid: *const c_void) -> *const c_void;
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
    static kCFBooleanTrue: *const c_void;
    static kCFBooleanFalse: *const c_void;
    fn CFBooleanGetValue(value: *const c_void) -> u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_remote_calls_the_token_constructor() {
        let mut api = AxApi::new(&crate::private_api::PrivateApi::live());
        let mut cursor = 0u64;
        let found = resolve_remote(&mut api, 1, std::process::id() as i32, &mut cursor, 1);
        assert_eq!(cursor, 1);
        assert!(!found);
    }

    #[test]
    fn raise_fails_when_the_element_is_unknown() {
        let api = AxApi::new(&crate::private_api::PrivateApi::live());
        assert!(!raise(&api, 1, 1));
    }
}
