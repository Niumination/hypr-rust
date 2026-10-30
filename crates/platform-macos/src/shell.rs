//! Minimal macOS menu-bar shell (Gap 3).
//!
//! A Rust `NSStatusItem` with a menu: a status line, explicit reflow commands,
//! pause/resume, an Accessibility status line, diagnostics, and quit. All
//! AppKit/Objective-C `unsafe` FFI lives in [`crate::ffi`]; this module only
//! composes documented wrappers behind a small surface.
//!
//! Auto-tiling stays **off**: nothing is applied unless the user picks a reflow
//! command. Quit terminates the app and leaves windows at their last
//! OS-accepted position. The shell never reads window titles.
//!
//! Two reflow surfaces (ADR-0006): the interactive **menu** command targets
//! real app windows — every normal window of every regular app on the main
//! display, strict no-partial — while every **automated** path (`--self-test`,
//! `--panel-click-proof`) keeps calling the fixture-bound [`perform_reflow`],
//! so an evidence run can never move an owner's real windows.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Mutex;

use layout_core::{DisplayId, Rect};

use crate::adapter::{
    global_window_key, split_global_window_key, AxError, MacAxAdapter, MacMultiWindowSet,
};
use crate::batch;
use crate::config::{dialog_policy_for, rules_json, Config, ConfigError, DialogPolicy, WindowRule};
use crate::ffi::{self, CGFloat};
use crate::{fixture, process_alive};

/// Objective-C object / selector handles. Both are opaque pointers.
type Id = *const std::os::raw::c_void;
type Sel = *const std::os::raw::c_void;

/// Raw Objective-C handle kept in [`ShellState`].
///
/// The shell runs on one thread (the main run loop) and touches these handles
/// only there, so the handle is `Send` even though a bare raw pointer is not.
#[derive(Clone, Copy)]
struct Handle(Id);

unsafe impl Send for Handle {}

impl Handle {
    const fn null() -> Self {
        Self(std::ptr::null())
    }

    fn is_null(self) -> bool {
        self.0.is_null()
    }
}

// MARK: - Objective-C message-send helpers
//
// `objc_msgSend` is variadic in C and Rust rejects redeclaring one symbol with
// different signatures in a crate, so there is a single declaration in `ffi.rs`
// and each arity is produced here by transmuting it to a concrete signature.
// All raw FFI stays in `ffi.rs`.

macro_rules! objc_msg {
    ($name:ident($($arg:ident : $ty:ty),*)) => {
        /// Send an Objective-C message.
        ///
        /// SAFETY: `obj` must be a live object (or null), `s` a registered
        /// selector, and the arguments must match the selector's types.
        unsafe fn $name(obj: Id, s: Sel $(, $arg: $ty)*) -> Id {
            let f: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> Id = unsafe {
                std::mem::transmute(ffi::objc_msgSend as unsafe extern "C" fn(Id, Sel) -> Id)
            };
            unsafe { f(obj, s $(, $arg)*) }
        }
    };
}

objc_msg!(msg_id(a: Id));
objc_msg!(msg_bool(a: bool));
objc_msg!(msg_int(a: isize));
objc_msg!(msg_cgfloat(a: CGFloat));
objc_msg!(msg_i32(a: i32));
objc_msg!(msg_id_id_id_id(a: Id, b: Id, c: Id, d: Id));
objc_msg!(msg_view_init(rect: ffi::CGRect));
objc_msg!(msg_popup_init(rect: ffi::CGRect, pulls_down: bool));
objc_msg!(msg_window_init(
    rect: ffi::CGRect,
    style_mask: isize,
    backing: i32,
    defer_flag: u8
));
objc_msg!(msg_id_len(idx: usize));

/// Send an Objective-C message with a non-object return type.
///
/// SAFETY: see [`msg0`].
macro_rules! objc_msg_ret {
    ($name:ident($($arg:ident : $ty:ty),*) -> $ret:ty) => {
        unsafe fn $name(obj: Id, s: Sel $(, $arg: $ty)*) -> $ret {
            let f: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> $ret = unsafe {
                std::mem::transmute(ffi::objc_msgSend as unsafe extern "C" fn(Id, Sel) -> Id)
            };
            unsafe { f(obj, s $(, $arg)*) }
        }
    };
}

objc_msg_ret!(msg_isize() -> isize);
objc_msg_ret!(msg_u8() -> u8);
objc_msg_ret!(msg_u8_id(a: Id) -> u8);
objc_msg_ret!(msg_cstr() -> *const c_char);
objc_msg_ret!(msg_set_frame(frame: ffi::CGRect) -> ());

/// Send a message with no arguments.
///
/// SAFETY: `obj` must be a live object (or null) and `s` a registered selector.
unsafe fn msg0(obj: Id, s: Sel) -> Id {
    unsafe { ffi::objc_msgSend(obj, s) }
}

/// Send a message whose return value is not needed.
///
/// SAFETY: see [`msg0`].
unsafe fn msg_void(obj: Id, s: Sel) {
    unsafe { msg0(obj, s) };
}

/// Send a one-argument message whose return value is not needed.
///
/// SAFETY: see [`msg0`].
unsafe fn msg_void_id(obj: Id, s: Sel, a: Id) {
    unsafe { msg_id(obj, s, a) };
}

/// SAFETY: `name` must be a NUL-terminated class name.
unsafe fn class(name: &str) -> Id {
    let c = CString::new(name).expect("class name is NUL-free");
    unsafe { ffi::objc_getClass(c.as_ptr()) }
}

/// SAFETY: `name` must be a NUL-terminated selector.
unsafe fn sel(name: &str) -> Sel {
    let c = CString::new(name).expect("selector is NUL-free");
    unsafe { ffi::sel_registerName(c.as_ptr()) }
}

/// Build an `NSString`. The app leaks one at startup and one per menu refresh;
/// this is a session-long menu-bar app, so that is intentional and bounded by
/// user actions.
///
/// SAFETY: `class("NSString")` must resolve.
unsafe fn nsstring(text: &str) -> Id {
    let c = CString::new(text).expect("menu text is NUL-free");
    // SAFETY: NSString is a live class and the argument is a valid C string.
    unsafe {
        msg_id(
            class("NSString"),
            sel("stringWithUTF8String:"),
            c.as_ptr() as Id,
        )
    }
}

// MARK: - Shell state and actions

/// Handles for the settings panel's AppKit objects (built once, kept for the
/// session, main-thread only — same lifetime rationale as the menu).
#[derive(Clone, Copy)]
struct SettingsPanel {
    window: Handle,
    popup: Handle,
    outer: Handle,
    inner: Handle,
    ratio: Handle,
    auto_tiling: Handle,
    dialogs: Handle,
    rows_area: Handle,
    save: Handle,
    /// Top strip text line: mirrors the save/invalid status so the outcome
    /// is visible in the panel itself, not only on the menu chip
    /// (owner decision 2026-10-26).
    status: Handle,
}

// Field layout inside the panel content view (content coordinates, origin at
// the bottom-left; the title bar sits above). The rules area holds the
// per-rule rows (bundle entry + policy popup + remove per row); Save saves
// what is on screen, rules from the rows.
const K_SETTINGS_RECTS: [(f64, f64, f64, f64); 17] = [
    (16.0, 292.0, 88.0, 20.0),   // label "Layout"
    (110.0, 288.0, 150.0, 24.0), // popup
    (16.0, 260.0, 88.0, 20.0),   // label "Outer gap"
    (110.0, 258.0, 70.0, 20.0),  // outer field
    (16.0, 228.0, 88.0, 20.0),   // label "Inner gap"
    (110.0, 226.0, 70.0, 20.0),  // inner field
    (16.0, 196.0, 88.0, 20.0),   // label "Master ratio"
    (110.0, 194.0, 70.0, 20.0),  // ratio field
    (16.0, 40.0, 100.0, 18.0),   // checkbox "Auto-tiling"
    (120.0, 40.0, 140.0, 18.0),  // checkbox "Include dialogs"
    (110.0, 8.0, 70.0, 22.0),    // Save
    (190.0, 8.0, 70.0, 22.0),    // Close
    (200.0, 292.0, 120.0, 20.0), // label "changes on Save"
    (16.0, 170.0, 190.0, 18.0),  // label "Rules (JSON array)"
    (16.0, 66.0, 308.0, 96.0),   // rules row area
    (210.0, 166.0, 100.0, 22.0), // Add rule
    (16.0, 308.0, 308.0, 16.0),  // status line (Save outcomes; cleared on open)
];

/// Handle kinds for [`create_settings_panel`].
const K_SETTINGS_WINDOW_W: CGFloat = 340.0;
const K_SETTINGS_WINDOW_H: CGFloat = 330.0;

/// `NSWindowStyleMask.titled | closable | miniaturizable`.
const K_SETTINGS_STYLE_MASK: isize = 1 | 2 | 4;
/// `NSBackingStoreBuffered` (a C enum, passed as an `int`).
const K_BACKING_BUFFERED: i32 = 2;
/// `NSButtonTypeSwitch` (checkbox; a C enum, passed as an `int`).
const K_BUTTON_TYPE_SWITCH: i32 = 3;
/// `NSButtonTypeMomentaryPushIn` (regular button).
const K_BUTTON_TYPE_PUSH: i32 = 0;
/// `NSControlStateValueOn`.
const K_STATE_ON: isize = 1;

/// Shell state shared with the Objective-C action callbacks. Menu actions run
/// on the main thread, so a plain `Mutex` is enough.
struct ShellState {
    paused: bool,
    app: Handle,
    status_item: Handle,
    target: Handle,
    config: Option<Config>,
    settings: Option<SettingsPanel>,
    status: String,
}

static STATE: Mutex<ShellState> = Mutex::new(ShellState {
    paused: false,
    app: Handle::null(),
    status_item: Handle::null(),
    target: Handle::null(),
    config: None,
    settings: None,
    status: String::new(),
});

/// One interactive rules row in the settings panel: the row's host view, its
/// entry (an editable combo for bundle rows; a read-only label for pid rules,
/// which only a JSON file can express), the dialog-policy popup and the
/// row's remove button (identified by pointer, not by an integer tag, so no
/// re-tagging is needed when rows shift). `pid` records the pid the row
/// round-trips for pid rows.
#[derive(Clone, Copy)]
struct RuleRow {
    view: Handle,
    entry: Handle,
    policy: Handle,
    remove: Handle,
    pid: Option<u32>,
}

/// Row state lives outside `ShellState` so panel code (populate, add,
/// remove, save) can read it without re-entrant locks. Lock order is fixed
/// everywhere: `STATE` strictly before `RULE_ROWS`, never the reverse.
static RULE_ROWS: Mutex<Vec<RuleRow>> = Mutex::new(Vec::new());

/// Policy popup items, in display order.
const K_POLICY_ITEMS: [&str; 3] = ["include", "exclude", "default"];

/// The dialog policy a popup title maps to. Anything unrecognised — including
/// no selection — stays on the conservative default.
fn policy_from_title(title: Option<&str>) -> DialogPolicy {
    match title {
        Some("include") => DialogPolicy::Include,
        Some("exclude") => DialogPolicy::Exclude,
        _ => DialogPolicy::Default,
    }
}

/// The loaded config, or the defaults before one is loaded.
fn current_config() -> Config {
    STATE.lock().unwrap().config.clone().unwrap_or_default()
}

/// The current rules (ADR-0008: the auto-tiling trigger filter re-reads
/// them per event, because the panel Save can change them at runtime).
pub(crate) fn current_rules() -> Vec<WindowRule> {
    current_config().rules
}

/// Load the config, falling back to defaults. A missing file is normal
/// (defaults); anything else is reported to the log without being fatal.
fn load_config_or_default() -> Config {
    match Config::load() {
        Ok(config) => config,
        Err(error) => {
            if !matches!(error, ConfigError::Missing) {
                append_log(&format!("config: {error}"));
            }
            Config::default()
        }
    }
}

extern "C" fn action_reflow(_this: Id, _cmd: Sel) {
    // The menu is the non-fixture, owner-driven surface (ADR-0006 §3): it
    // targets real app windows. Automated evidence keeps calling
    // `perform_reflow` (fixture-bound) instead.
    perform_reflow_real();
}

extern "C" fn action_toggle_pause(_this: Id, _cmd: Sel) {
    {
        let mut state = STATE.lock().unwrap();
        state.paused = !state.paused;
    }
    refresh_menu();
}

extern "C" fn action_reset_defaults(_this: Id, _cmd: Sel) {
    let defaults = Config::default();
    match defaults.save() {
        Ok(()) => {
            // Adopt the defaults immediately so the menu, the next reflow and
            // the diagnostics never act on a config the file no longer contains.
            STATE.lock().unwrap().config = Some(defaults);
            set_status("settings reset to defaults");
        }
        Err(error) => set_status(&format!("reset failed: {error}")),
    }
    refresh_menu();
}

/// Settings implementations shared with the self-test: the selector bodies
/// below call these, so programmatic self-test input exercises the same code
/// path minus the Objective-C dispatch.
fn save_settings_from_panel() {
    let panel = {
        let state = STATE.lock().unwrap();
        match state.settings {
            Some(ref settings) => *settings,
            None => return,
        }
    };
    // SAFETY: the panel handles are live AppKit objects on the main thread.
    let (popup, outer, inner, ratio, auto, dialogs) = unsafe {
        (
            popup_title(panel.popup),
            field_text(panel.outer),
            field_text(panel.inner),
            field_text(panel.ratio),
            checkbox_on(panel.auto_tiling),
            checkbox_on(panel.dialogs),
        )
    };
    // Field-level number validation before the body is built (owner decision
    // 2026-10-26): a comma decimal separator or stray characters are refused
    // with the field named and the expected shape, instead of surfacing later
    // as a generic JSON error. Range checks stay in `Config` (0.05–0.95 and
    // ≥ 0); this step only decides whether the text is a number at all.
    for (name, text) in [
        ("outer_gap", outer.as_str()),
        ("inner_gap", inner.as_str()),
        ("master_ratio", ratio.as_str()),
    ] {
        let trimmed = text.trim();
        if trimmed.is_empty() || !is_json_number(trimmed) {
            set_settings_status(
                panel,
                &format!("invalid number in {name}: {trimmed:?} (use digits and a dot, e.g. 0.35)"),
            );
            return;
        }
    }
    // The rows are the authority for rules: the panel saves only the
    // canonical regenerated JSON built from them — raw entry text never
    // reaches the body, and the final `Config::from_json` validation (the
    // same entry validation a file load runs) rejects a matcher-less row, so
    // a row can never silently match every app.
    // SAFETY: the rows hold live AppKit handles; main thread (the save path
    // runs only from the button action, the self-test or the proof).
    let rules = unsafe { rules_from_rows() };
    let body = settings_body(
        popup.as_deref().unwrap_or(""),
        &outer,
        &inner,
        &ratio,
        auto,
        dialogs,
        &rules_json(&rules),
    );
    match Config::from_json(&body) {
        Ok(config) => match config.save() {
            Ok(()) => {
                let was_enabled = STATE
                    .lock()
                    .unwrap()
                    .config
                    .as_ref()
                    .map(|loaded| loaded.auto_tiling)
                    .unwrap_or(false);
                let now_enabled = config.auto_tiling;
                STATE.lock().unwrap().config = Some(config);
                set_settings_status(panel, "settings saved");
                // ADR-0008 §1: the toggle applies live — re-sync the
                // observers with the new rules; enabling also runs one
                // immediate strict reflow so the effect is visible without
                // hunting for a window event. Automated-evidence paths are
                // a no-op inside set_enabled (§5).
                crate::auto_tiling::set_enabled(now_enabled);
                if now_enabled && !was_enabled {
                    perform_reflow_real();
                }
            }
            Err(error) => {
                set_settings_status(panel, &format!("settings save failed: {error}"));
            }
        },
        Err(error) => {
            // A parse failure means the generated body itself is malformed;
            // record it verbatim (numbers/bools/rule matchers only — no
            // titles) so the exact shape is diagnosable from the log.
            if matches!(error, ConfigError::Parse) {
                append_log(&format!("settings body (malformed): {body}"));
            }
            set_settings_status(panel, &format!("settings invalid: {error}"));
        }
    }
}

/// Publish a settings outcome where the user is looking: the menu chip and
/// the panel's own status line (owner decision 2026-10-26 — the panel used
/// to stay silent and the chip was easy to miss).
fn set_settings_status(panel: SettingsPanel, status: &str) {
    // SAFETY: live NSTextField on the main thread (the save path runs only
    // from the button action, the self-test or the proof).
    unsafe { set_field_text(panel.status, status) };
    set_status(status);
}

/// Strict JSON-number check: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?\d+)?`.
///
/// The panel's body builder (`number_text`) only screens characters, so a
/// value like `.35`, `0.35e` or `0.35.` still passes that guard and breaks
/// the generated body (observed on the owner's host as "config is not valid
/// JSON"). This refuses everything the JSON parser cannot read as a number;
/// range validation remains with `Config`.
fn is_json_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    if index < bytes.len() && bytes[index] == b'-' {
        index += 1;
    }
    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == integer_start {
        return false;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        index += 1;
        if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

/// Open (and on first use build) the settings panel, populated from the
/// effective config.
fn show_settings() {
    let mut state = STATE.lock().unwrap();
    if state.settings.is_none() {
        let target = state.target.0;
        // SAFETY: main thread; the app and the action target are live.
        state.settings = Some(unsafe { create_settings_panel(target) });
    }
    let panel = state.settings.unwrap();
    let config = state.config.clone().unwrap_or_default();
    let app = state.app;
    let target = state.target.0;
    // SAFETY: main-thread calls on live handles; populate before showing.
    unsafe {
        populate_settings(panel, &config, target);
        if !app.is_null() {
            msg_bool(app.0, sel("activateIgnoringOtherApps:"), true);
        }
        msg_void(panel.window.0, sel("makeKeyAndOrderFront:"));
    }
}

extern "C" fn action_show_settings(_this: Id, _cmd: Sel) {
    show_settings();
}

extern "C" fn action_save_settings(_this: Id, _cmd: Sel) {
    save_settings_from_panel();
}

extern "C" fn action_close_settings(_this: Id, _cmd: Sel) {
    // SAFETY: hides a live window without closing the app.
    let window = {
        let state = STATE.lock().unwrap();
        state.settings.map(|panel| panel.window)
    };
    if let Some(window) = window {
        unsafe { msg_void_id(window.0, sel("orderOut:"), std::ptr::null()) };
    }
}

extern "C" fn action_diagnostics(_this: Id, _cmd: Sel) {
    let state = STATE.lock().unwrap();
    let config = state.config.clone().unwrap_or_default();
    let line = format!(
        "diagnostics: {} \u{b7} {}",
        status_line_with(&state),
        settings_line_with(&config)
    );
    drop(state);
    append_log(&line);
}

extern "C" fn action_quit(_this: Id, _cmd: Sel) {
    // ADR-0007: release the hotkey refs before the app goes away (the OS
    // would free them at exit anyway; explicit teardown keeps it honest).
    if let Some(error) = crate::hotkey::unregister() {
        append_log(&format!("hotkey: quit teardown: {error}"));
    }
    // ADR-0008 §4: explicit observer teardown on quit — remove every
    // notification, run-loop source and owned ref before the app goes away.
    crate::auto_tiling::set_enabled(false);
    let app = STATE.lock().unwrap().app;
    if !app.is_null() {
        // SAFETY: `app` is the live NSApplication instance.
        unsafe { msg_void_id(app.0, sel("terminate:"), std::ptr::null()) };
    }
}

/// NSWorkspace app launch/terminate notifications (registered in `run()`,
/// ADR-0008 §2): attach or drop the per-app observer. Delivered on the
/// main thread; a no-op while auto-tiling is off.
extern "C" fn action_workspace_app_event(_this: Id, _cmd: Sel, notification: Id) {
    // SAFETY: `notification` is a live NSNotification on the main thread;
    // the names and the userInfo key are the exported AppKit constants.
    unsafe {
        let name = msg0(notification, sel("name"));
        let is_launch = msg_u8_id(
            name,
            sel("isEqualToString:"),
            ffi::NSWorkspaceDidLaunchApplicationNotification as Id,
        ) != 0;
        let is_terminate = msg_u8_id(
            name,
            sel("isEqualToString:"),
            ffi::NSWorkspaceDidTerminateApplicationNotification as Id,
        ) != 0;
        if !is_launch && !is_terminate {
            return;
        }
        let info = msg0(notification, sel("userInfo"));
        if info.is_null() {
            return;
        }
        let app = msg_id(
            info,
            sel("objectForKey:"),
            ffi::NSWorkspaceApplicationKey as Id,
        );
        if app.is_null() {
            return;
        }
        let pid = msg_isize(app, sel("processIdentifier"));
        if pid <= 0 {
            return;
        }
        if is_launch {
            crate::auto_tiling::on_app_launched(pid as u32);
        } else {
            crate::auto_tiling::on_app_terminated(pid as u32);
        }
    }
}

fn settings_body(
    layout: &str,
    outer: &str,
    inner: &str,
    ratio: &str,
    auto_tiling: bool,
    include_dialogs: bool,
    rules_body: &str,
) -> String {
    // The layout name is embedded only when it is a known kind (otherwise an
    // empty string, which the parser rejects), and number text only when it
    // consists of number characters (otherwise `null`, rejected as unparseable).
    // Nothing in this path is attacker-relevant anyway — the panel edits the
    // user's own local file — but this keeps the JSON well-formed for arbitrary
    // typing and puts every final value through the same validation as a file
    // load.
    let layout = if layout_kind_name_lookup(layout).is_some() {
        layout
    } else {
        ""
    };
    format!(
        "{{\n  \"version\": {},\n  \"layout\": \"{layout}\",\n  \"outer_gap\": {},\n  \"inner_gap\": {},\n  \"master_ratio\": {},\n  \"auto_tiling\": {},\n  \"include_dialogs\": {},\n  \"rules\": {rules_body}\n}}\n",
        crate::config::CONFIG_VERSION,
        number_text(outer),
        number_text(inner),
        number_text(ratio),
        auto_tiling,
        include_dialogs,
    )
}

fn layout_kind_name_lookup(text: &str) -> Option<layout_core::LayoutKind> {
    crate::config::LAYOUT_KINDS
        .iter()
        .find(|(name, _)| *name == text)
        .map(|(_, kind)| *kind)
}

/// A number will only be written where the characters are unambiguous
/// number characters; anything else becomes `null`, which the parser rejects.
fn number_text(text: &str) -> String {
    let clean = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    if clean {
        text.to_string()
    } else {
        "null".to_string()
    }
}

// MARK: - Settings panel (AppKit)

/// Text of an editable field.
///
/// SAFETY: `field` must be a live NSTextField.
unsafe fn field_text(field: Handle) -> String {
    let value = unsafe { msg0(field.0, sel("stringValue")) };
    unsafe { utf8_string(value) }
}

/// Title of the popup's selected item, or `None` when nothing is selected.
///
/// SAFETY: `popup` must be a live NSPopUpButton.
unsafe fn popup_title(popup: Handle) -> Option<String> {
    let value = unsafe { msg0(popup.0, sel("titleOfSelectedItem")) };
    if value.is_null() {
        None
    } else {
        Some(unsafe { utf8_string(value) })
    }
}

/// On/off state of a checkbox.
///
/// SAFETY: `button` must be a live NSButton (switch type).
unsafe fn checkbox_on(button: Handle) -> bool {
    unsafe { msg_isize(button.0, sel("state")) == K_STATE_ON }
}

/// Set an editable field.
///
/// SAFETY: `field` must be a live NSTextField.
unsafe fn set_field_text(field: Handle, text: &str) {
    unsafe { msg_void_id(field.0, sel("setStringValue:"), nsstring(text)) };
}

/// Select a popup item by title (no-op for unknown titles).
///
/// SAFETY: `popup` must be a live NSPopUpButton and the title must exist.
unsafe fn select_popup_title(popup: Handle, title: &str) {
    unsafe {
        msg_void_id(popup.0, sel("selectItemWithTitle:"), nsstring(title));
    }
}

/// Set a checkbox state.
///
/// SAFETY: `button` must be a live NSButton (switch type).
unsafe fn set_checkbox(button: Handle, on: bool) {
    unsafe { msg_int(button.0, sel("setState:"), i64::from(on) as isize) };
}

/// Turn an NSString into a Rust string.
///
/// SAFETY: `value` must be a live NSString (or null; yields an empty string).
unsafe fn utf8_string(value: Id) -> String {
    if value.is_null() {
        return String::new();
    }
    let bytes = unsafe { msg_cstr(value, sel("UTF8String")) };
    if bytes.is_null() {
        return String::new();
    }
    // SAFETY: bytes is a NUL-terminated UTF-8 buffer owned by the string.
    unsafe { std::ffi::CStr::from_ptr(bytes) }
        .to_string_lossy()
        .into_owned()
}

/// Bundle id of a running app via public `NSRunningApplication`. Unsigned CLI
/// apps legitimately report no bundle id. This is app **identity** used for
/// user rules; window titles are never read anywhere in the shell.
///
/// SAFETY: AppKit class/instance selectors on the main thread.
unsafe fn app_bundle_id(pid: u32) -> Option<String> {
    let app = unsafe {
        msg_i32(
            class("NSRunningApplication"),
            sel("runningApplicationWithProcessIdentifier:"),
            pid as i32,
        )
    };
    if app.is_null() {
        return None;
    }
    let bundle = unsafe { msg0(app, sel("bundleIdentifier")) };
    let text = unsafe { utf8_string(bundle) };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Populate the panel from `config`.
///
/// SAFETY: the panel handles must be live AppKit objects; the caller holds
/// `STATE` (this function locks only `RULE_ROWS`, strictly after it).
unsafe fn populate_settings(panel: SettingsPanel, config: &Config, target: Id) {
    unsafe {
        select_popup_title(panel.popup, config.kind_name());
        set_field_text(panel.outer, &fmt_num(config.layout.outer_gap));
        set_field_text(panel.inner, &fmt_num(config.layout.inner_gap));
        set_field_text(panel.ratio, &fmt_num(config.layout.master_ratio));
        set_checkbox(panel.auto_tiling, config.auto_tiling);
        set_checkbox(panel.dialogs, config.include_dialogs);
        // The rows are the authority for rules: every open rebuilds them from
        // the effective config; Save re-validates whatever the rows hold (the
        // same entry validation a file load runs).
        rebuild_rules_rows(panel.rows_area, &config.rules, target);
        // A fresh open starts with a silent status line; outcomes are written
        // by the save path.
        set_field_text(panel.status, "");
    }
}

/// Build the settings window once.
///
/// SAFETY: main-thread only, after the NSApplication exists; the returned
/// handles are kept for the session in `ShellState.settings`.
unsafe fn create_settings_panel(target: Id) -> SettingsPanel {
    // SAFETY: documented class initialisers; window retains its subviews.
    let window = unsafe {
        let alloc = msg0(class("NSWindow"), sel("alloc"));
        msg_window_init(
            alloc,
            sel("initWithContentRect:styleMask:backing:defer:"),
            ffi::CGRect {
                origin: ffi::CGPoint { x: 300.0, y: 340.0 },
                size: ffi::CGSize {
                    width: K_SETTINGS_WINDOW_W,
                    height: K_SETTINGS_WINDOW_H,
                },
            },
            K_SETTINGS_STYLE_MASK,
            K_BACKING_BUFFERED,
            0,
        )
    };
    assert!(!window.is_null(), "settings window unavailable");
    let content = unsafe { msg0(window, sel("contentView")) };
    assert!(!content.is_null(), "settings content view unavailable");
    unsafe {
        msg_void_id(window, sel("setTitle:"), nsstring("Hypr-Rust settings"));
        msg_bool(window, sel("setReleasedWhenClosed:"), false);

        // Layout picker.
        let popup = {
            let alloc = msg0(class("NSPopUpButton"), sel("alloc"));
            msg_popup_init(
                alloc,
                sel("initWithFrame:pullsDown:"),
                rect(K_SETTINGS_RECTS[1]),
                false,
            )
        };
        for (name, _) in crate::config::LAYOUT_KINDS {
            msg_void_id(popup, sel("addItemWithTitle:"), nsstring(name));
        }
        msg_void_id(content, sel("addSubview:"), popup);

        let outer = make_field(content, K_SETTINGS_RECTS[3]);
        let inner = make_field(content, K_SETTINGS_RECTS[5]);
        let ratio = make_field(content, K_SETTINGS_RECTS[7]);

        let auto_tiling = make_checkbox(content, K_SETTINGS_RECTS[8], "Auto-tiling");
        let dialogs = make_checkbox(content, K_SETTINGS_RECTS[9], "Include dialogs");

        // Labels.
        make_label(content, K_SETTINGS_RECTS[0], "Layout");
        make_label(content, K_SETTINGS_RECTS[2], "Outer gap");
        make_label(content, K_SETTINGS_RECTS[4], "Inner gap");
        make_label(content, K_SETTINGS_RECTS[6], "Master ratio");
        make_label(content, K_SETTINGS_RECTS[12], "changes on Save");

        // Rules rows: a scrollable area (one row per rule: bundle entry +
        // policy popup + remove) plus the Add-rule button.
        make_label(content, K_SETTINGS_RECTS[13], "Rules");
        let rows_area = make_row_area(content, K_SETTINGS_RECTS[14]);
        let _add_rule = make_button(
            content,
            K_SETTINGS_RECTS[15],
            "Add rule",
            target,
            "addRule:",
        );

        // Save / Close: the window retains them; the actions live on `target`.
        let save = make_button(
            content,
            K_SETTINGS_RECTS[10],
            "Save",
            target,
            "saveSettings:",
        );
        let _close = make_button(
            content,
            K_SETTINGS_RECTS[11],
            "Close",
            target,
            "closeSettings:",
        );

        // Status line at the top strip: the save outcome also appears here so
        // the user does not have to open the menu to see what happened.
        let status = make_label(content, K_SETTINGS_RECTS[16], "");

        SettingsPanel {
            window: Handle(window),
            popup: Handle(popup),
            outer,
            inner,
            ratio,
            auto_tiling,
            dialogs,
            rows_area,
            save,
            status,
        }
    }
}

// MARK: - Rules rows (per-rule editor)

/// The title a dialog policy shows as the policy popup item.
fn policy_title(policy: DialogPolicy) -> &'static str {
    match policy {
        DialogPolicy::Include => "include",
        DialogPolicy::Exclude => "exclude",
        DialogPolicy::Default => "default",
    }
}

/// Bundle ids of currently running apps, for the bundle entries' completion
/// list. Only `bundleIdentifier`s are read — never window titles.
fn running_bundle_ids() -> Vec<String> {
    unsafe {
        let workspace = msg0(class("NSWorkspace"), sel("sharedWorkspace"));
        let apps = msg0(workspace, sel("runningApplications"));
        if apps.is_null() {
            return Vec::new();
        }
        let count = msg_isize(apps, sel("count"));
        let mut ids = Vec::new();
        for index in 0..count.max(0) {
            let app = msg_id_len(apps, sel("objectAtIndex:"), index as usize);
            if app.is_null() {
                continue;
            }
            let bundle = msg0(app, sel("bundleIdentifier"));
            if bundle.is_null() {
                continue;
            }
            ids.push(utf8_string(bundle));
        }
        ids.sort();
        ids.dedup();
        ids
    }
}

/// Pids of running **regular** apps (`NSApplicationActivationPolicyRegular
/// == 0`), sorted for deterministic enumeration. Background and agent apps
/// own no normal windows and are skipped. Only process identifiers and
/// activation policies are read — never window titles.
pub(crate) fn regular_app_pids() -> Vec<u32> {
    unsafe {
        let workspace = msg0(class("NSWorkspace"), sel("sharedWorkspace"));
        let apps = msg0(workspace, sel("runningApplications"));
        if apps.is_null() {
            return Vec::new();
        }
        let count = msg_isize(apps, sel("count"));
        let mut pids = Vec::new();
        for index in 0..count.max(0) {
            let app = msg_id_len(apps, sel("objectAtIndex:"), index as usize);
            if app.is_null() {
                continue;
            }
            if msg_isize(app, sel("activationPolicy")) != 0 {
                continue;
            }
            let pid = msg_isize(app, sel("processIdentifier"));
            if pid > 0 {
                pids.push(pid as u32);
            }
        }
        pids.sort_unstable();
        pids.dedup();
        pids
    }
}

/// The bundle identifier of the running app with `pid`, if it has one
/// (unsigned CLI apps legitimately report none). Read-only: process id and
/// bundle identifier only — never window titles (ADR-0008: the auto-tiling
/// trigger filter needs the bundle to apply `exclude` rules).
pub(crate) fn bundle_id_for_pid(pid: u32) -> Option<String> {
    unsafe {
        let workspace = msg0(class("NSWorkspace"), sel("sharedWorkspace"));
        let apps = msg0(workspace, sel("runningApplications"));
        if apps.is_null() {
            return None;
        }
        let count = msg_isize(apps, sel("count"));
        for index in 0..count.max(0) {
            let app = msg_id_len(apps, sel("objectAtIndex:"), index as usize);
            if app.is_null() || msg_isize(app, sel("processIdentifier")) as u32 != pid {
                continue;
            }
            let bundle = msg0(app, sel("bundleIdentifier"));
            return if bundle.is_null() {
                None
            } else {
                Some(utf8_string(bundle))
            };
        }
        None
    }
}

/// The frontmost app's pid at command time — the master source of ADR-0006
/// §4 — or `None` when it cannot be read. Only the process identifier is
/// read.
fn frontmost_pid() -> Option<u32> {
    unsafe {
        let workspace = msg0(class("NSWorkspace"), sel("sharedWorkspace"));
        let app = msg0(workspace, sel("frontmostApplication"));
        if app.is_null() {
            return None;
        }
        let pid = msg_isize(app, sel("processIdentifier"));
        if pid > 0 {
            Some(pid as u32)
        } else {
            None
        }
    }
}

/// Build the scrollable rules area and return its document view handle (one
/// row per rule is laid out inside it; the scroll view itself is retained by
/// the window — the same recipe the old text editor used).
///
/// SAFETY: main thread; `content` must be a live view.
unsafe fn make_row_area(content: Id, place: (f64, f64, f64, f64)) -> Handle {
    unsafe {
        let scroll = {
            let alloc = msg0(class("NSScrollView"), sel("alloc"));
            let scroll = msg_view_init(alloc, sel("initWithFrame:"), rect(place));
            msg_bool(scroll, sel("setHasVerticalScroller:"), true);
            msg_bool(scroll, sel("setAutohidesScrollers:"), true);
            scroll
        };
        let doc = {
            let alloc = msg0(class("NSView"), sel("alloc"));
            msg_view_init(alloc, sel("initWithFrame:"), rect((0.0, 0.0, 308.0, 96.0)))
        };
        msg_void_id(scroll, sel("setDocumentView:"), doc);
        msg_void_id(content, sel("addSubview:"), scroll);
        Handle(doc)
    }
}

/// Build one rules row and add it to `doc` at `y` (bottom-left origin,
/// matching the panel's coordinate convention). A bundle row gets an editable
/// combo seeded with the running apps' bundle ids; a pid row gets a read-only
/// label (pid rules can only come from a JSON file) and round-trips its pid.
///
/// SAFETY: main thread; `doc` and `target` must be live.
unsafe fn make_rule_row(
    doc: Id,
    y: f64,
    pid: Option<u32>,
    bundle: Option<&str>,
    policy: DialogPolicy,
    target: Id,
    suggestions: &[String],
) -> RuleRow {
    unsafe {
        let view = {
            let alloc = msg0(class("NSView"), sel("alloc"));
            msg_view_init(alloc, sel("initWithFrame:"), rect((0.0, y, 308.0, 22.0)))
        };
        // A pid row is a read-only label (pid rules can only come from a
        // JSON file); a bundle row is an editable combo seeded with the
        // running apps' bundle ids.
        let entry = if let Some(shown_pid) = pid {
            let label = {
                let alloc = msg0(class("NSTextField"), sel("alloc"));
                let label =
                    msg_view_init(alloc, sel("initWithFrame:"), rect((0.0, 0.0, 170.0, 22.0)));
                msg_bool(label, sel("setEditable:"), false);
                msg_bool(label, sel("setBordered:"), false);
                msg_bool(label, sel("setDrawsBackground:"), false);
                label
            };
            msg_void_id(
                label,
                sel("setStringValue:"),
                nsstring(&format!("pid {shown_pid} (from file)")),
            );
            label
        } else {
            let combo = {
                let alloc = msg0(class("NSComboBox"), sel("alloc"));
                let combo =
                    msg_view_init(alloc, sel("initWithFrame:"), rect((0.0, 0.0, 170.0, 22.0)));
                msg_bool(combo, sel("setUsesDataSource:"), false);
                for suggestion in suggestions {
                    msg_void_id(combo, sel("addItemWithObjectValue:"), nsstring(suggestion));
                }
                combo
            };
            if let Some(bundle_id) = bundle {
                msg_void_id(combo, sel("setStringValue:"), nsstring(bundle_id));
            }
            combo
        };
        msg_void_id(view, sel("addSubview:"), entry);
        let policy_popup = {
            let alloc = msg0(class("NSPopUpButton"), sel("alloc"));
            msg_popup_init(
                alloc,
                sel("initWithFrame:pullsDown:"),
                rect((176.0, 0.0, 90.0, 22.0)),
                false,
            )
        };
        for item in K_POLICY_ITEMS {
            msg_void_id(policy_popup, sel("addItemWithTitle:"), nsstring(item));
        }
        msg_void_id(
            policy_popup,
            sel("selectItemWithTitle:"),
            nsstring(policy_title(policy)),
        );
        msg_void_id(view, sel("addSubview:"), policy_popup);
        let remove = make_button(view, (272.0, 0.0, 26.0, 22.0), "–", target, "removeRule:");
        msg_void_id(doc, sel("addSubview:"), view);
        RuleRow {
            view: Handle(view),
            entry: Handle(entry),
            policy: Handle(policy_popup),
            remove,
            pid,
        }
    }
}

/// Replace the rule rows with one row per effective rule.
///
/// SAFETY: main thread; live handles; the caller holds `STATE` (this
/// function locks only `RULE_ROWS`, strictly after it).
unsafe fn rebuild_rules_rows(doc: Handle, rules: &[crate::config::WindowRule], target: Id) {
    let suggestions = running_bundle_ids();
    let old = RULE_ROWS.lock().unwrap().split_off(0);
    for row in old {
        msg_void(row.view.0, sel("removeFromSuperview"));
    }
    let mut rows = Vec::new();
    for rule in rules {
        rows.push(make_rule_row(
            doc.0,
            0.0,
            rule.pid,
            rule.bundle.as_deref(),
            rule.dialogs,
            target,
            &suggestions,
        ));
    }
    *RULE_ROWS.lock().unwrap() = rows;
    relayout_rules_rows(doc);
}

/// Lay the rows out top-down and size the document view to fit (the panel
/// shows 96 pt of rules; more rows scroll).
///
/// SAFETY: main thread; `doc` and the row handles are live.
unsafe fn relayout_rules_rows(doc: Handle) {
    let rows = RULE_ROWS.lock().unwrap();
    let height = std::cmp::max(96, rows.len() * 26) as f64;
    msg_set_frame(doc.0, sel("setFrame:"), rect((0.0, 0.0, 308.0, height)));
    for (index, row) in rows.iter().enumerate() {
        let y = height - 22.0 - (index as f64) * 26.0;
        msg_set_frame(row.view.0, sel("setFrame:"), rect((0.0, y, 308.0, 22.0)));
    }
}

/// Read the rules the rows currently express. A bundle row contributes its
/// entry text as the bundle (empty text = no matcher); a pid row round-trips
/// its pid unchanged. A row with neither matcher has no bundle and no pid —
/// the final `Config::from_json` validation rejects it (fail closed).
///
/// SAFETY: main thread; the row handles are live.
unsafe fn rules_from_rows() -> Vec<crate::config::WindowRule> {
    let rows = RULE_ROWS.lock().unwrap();
    let mut rules = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        let bundle = if row.pid.is_none() {
            match field_text(row.entry).trim() {
                "" => None,
                bundle_id => Some(bundle_id.to_string()),
            }
        } else {
            None
        };
        rules.push(crate::config::WindowRule {
            bundle,
            pid: row.pid,
            dialogs: policy_from_title(popup_title(row.policy).as_deref()),
        });
    }
    rules
}

/// Add one empty bundle row — the Add-rule action's code path, also used by
/// the self-test and the panel proof. Returns whether a row was added.
fn add_rule_row() -> bool {
    let doc_target = {
        let state = STATE.lock().unwrap();
        state.settings.map(|panel| (panel.rows_area, state.target))
    };
    match doc_target {
        None => false,
        Some((doc, target)) => {
            let suggestions = running_bundle_ids();
            // SAFETY: main thread; both handles are live.
            let row = unsafe {
                make_rule_row(
                    doc.0,
                    0.0,
                    None,
                    None,
                    DialogPolicy::Default,
                    target.0,
                    &suggestions,
                )
            };
            RULE_ROWS.lock().unwrap().push(row);
            // SAFETY: main thread; `doc` is live.
            unsafe { relayout_rules_rows(doc) };
            true
        }
    }
}

/// Add one pid rule row (a read-only label round-trips the pid; pid rules can
/// only come from a JSON file). Used by the self-test so the pid round-trip is
/// exercised at the row surface too. Returns whether a row was added.
fn add_pid_rule_row(pid: u32) -> bool {
    let doc_target = {
        let state = STATE.lock().unwrap();
        state.settings.map(|panel| (panel.rows_area, state.target))
    };
    match doc_target {
        None => false,
        Some((doc, target)) => {
            // SAFETY: main thread; both handles are live. The suggestions are
            // unused on a pid row.
            let row = unsafe {
                make_rule_row(
                    doc.0,
                    0.0,
                    Some(pid),
                    None,
                    DialogPolicy::Include,
                    target.0,
                    &[],
                )
            };
            RULE_ROWS.lock().unwrap().push(row);
            // SAFETY: main thread; `doc` is live.
            unsafe { relayout_rules_rows(doc) };
            true
        }
    }
}

/// Remove the row whose remove button is `sender` (pointer identity). This is
/// the remove button's action code path.
fn remove_rule_row(sender: Id) -> bool {
    let doc = {
        let state = STATE.lock().unwrap();
        state.settings.map(|panel| panel.rows_area)
    };
    let Some(doc) = doc else {
        return false;
    };
    let removed = {
        let mut rows = RULE_ROWS.lock().unwrap();
        rows.iter()
            .position(|row| row.remove.0 == sender)
            .map(|index| rows.remove(index))
    };
    let Some(row) = removed else {
        return false;
    };
    {
        let count = RULE_ROWS.lock().unwrap().len();
        append_log(&format!("panel: rules row removed (rows={count} left)"));
    }
    // SAFETY: main thread; `doc` is live.
    unsafe { msg_void(row.view.0, sel("removeFromSuperview")) };
    unsafe { relayout_rules_rows(doc) };
    true
}

extern "C" fn action_add_rule(_this: Id, _cmd: Sel) {
    if add_rule_row() {
        set_status("rules row added");
    } else {
        set_status("rules row add ignored (panel unavailable)");
    }
}

extern "C" fn action_remove_rule(_this: Id, _cmd: Sel, sender: Id) {
    if remove_rule_row(sender) {
        set_status("rules row removed");
    } else {
        set_status("rules row remove ignored (unknown row)");
    }
}

/// A text-field rectangle literal.
fn rect(item: (f64, f64, f64, f64)) -> ffi::CGRect {
    ffi::CGRect {
        origin: ffi::CGPoint {
            x: item.0,
            y: item.1,
        },
        size: ffi::CGSize {
            width: item.2,
            height: item.3,
        },
    }
}

/// Allocate/initialize an editable text field and add it to `content`.
///
/// SAFETY: main thread; `content` must be a live view.
unsafe fn make_field(content: Id, place: (f64, f64, f64, f64)) -> Handle {
    // SAFETY: documented initialisers.
    let field = unsafe {
        let alloc = msg0(class("NSTextField"), sel("alloc"));
        msg_view_init(alloc, sel("initWithFrame:"), rect(place))
    };
    unsafe { msg_void_id(content, sel("addSubview:"), field) };
    Handle(field)
}

/// Allocate/initialize a checkbox.
///
/// SAFETY: main thread; `content` must be a live view.
unsafe fn make_checkbox(content: Id, place: (f64, f64, f64, f64), title: &str) -> Handle {
    // SAFETY: documented initialisers; NSButtonTypeSwitch is the checkbox type.
    let button = unsafe {
        let alloc = msg0(class("NSButton"), sel("alloc"));
        let button = msg_view_init(alloc, sel("initWithFrame:"), rect(place));
        msg_i32(button, sel("setButtonType:"), K_BUTTON_TYPE_SWITCH);
        msg_void_id(button, sel("setTitle:"), nsstring(title));
        button
    };
    unsafe { msg_void_id(content, sel("addSubview:"), button) };
    Handle(button)
}

/// Allocate/initialize a push button bound to `action` on `target`.
///
/// SAFETY: main thread; `target` must be a live object whose class registered
/// the selector.
unsafe fn make_button(
    content: Id,
    place: (f64, f64, f64, f64),
    title: &str,
    target: Id,
    action: &str,
) -> Handle {
    // SAFETY: documented initialisers; NSButtonTypeMomentaryPushIn is the
    // standard push-button type.
    let button = unsafe {
        let alloc = msg0(class("NSButton"), sel("alloc"));
        let button = msg_view_init(alloc, sel("initWithFrame:"), rect(place));
        msg_i32(button, sel("setButtonType:"), K_BUTTON_TYPE_PUSH);
        msg_void_id(button, sel("setTitle:"), nsstring(title));
        msg_void_id(button, sel("setTarget:"), target);
        msg_void_id(button, sel("setAction:"), sel(action) as Id);
        button
    };
    unsafe { msg_void_id(content, sel("addSubview:"), button) };
    Handle(button)
}

/// Allocate/initialize a static label.
///
/// SAFETY: main thread; `content` must be a live view.
unsafe fn make_label(content: Id, place: (f64, f64, f64, f64), title: &str) -> Handle {
    // SAFETY: documented initialisers; disabling edit/border/background turns
    // a plain text field into a label.
    unsafe {
        let label = {
            let alloc = msg0(class("NSTextField"), sel("alloc"));
            msg_view_init(alloc, sel("initWithFrame:"), rect(place))
        };
        msg_bool(label, sel("setEditable:"), false);
        msg_bool(label, sel("setBordered:"), false);
        msg_bool(label, sel("setDrawsBackground:"), false);
        msg_void_id(label, sel("setStringValue:"), nsstring(title));
        msg_void_id(content, sel("addSubview:"), label);
        Handle(label)
    }
}

// MARK: - Menu construction

/// A disabled item used as a live status line.
///
/// SAFETY: `menu` is a live NSMenu.
unsafe fn add_status(menu: Id, text: &str) {
    // SAFETY: NSMenuItem/new and setTitle: are documented; a null action makes
    // the item inert.
    let item = unsafe { msg0(class("NSMenuItem"), sel("new")) };
    unsafe {
        msg_void_id(item, sel("setTitle:"), nsstring(text));
        msg_bool(item, sel("setEnabled:"), false);
        msg_void_id(menu, sel("addItem:"), item);
    }
}

/// An enabled item bound to `target` for `action`.
///
/// SAFETY: `menu` is a live NSMenu and `target` a live object.
unsafe fn add_action(menu: Id, target: Id, text: &str, action: &str) {
    // SAFETY: documented initialisers; the selector is registered on the target.
    let item = unsafe { msg0(class("NSMenuItem"), sel("new")) };
    unsafe {
        msg_void_id(item, sel("setTitle:"), nsstring(text));
        msg_void_id(item, sel("setAction:"), sel(action) as Id);
        msg_void_id(item, sel("setTarget:"), target);
        msg_void_id(menu, sel("addItem:"), item);
    }
}

/// A separator item.
///
/// SAFETY: NSMenuItem is a live class.
unsafe fn separator() -> Id {
    unsafe { msg0(class("NSMenuItem"), sel("separatorItem")) }
}

/// Build a fresh menu reflecting the current state.
///
/// SAFETY: `target` is the live action-target object.
unsafe fn build_menu(target: Id) -> Id {
    // SAFETY: documented class/instance initialisers.
    let menu = unsafe { msg0(class("NSMenu"), sel("new")) };
    let (config, paused) = {
        let state = STATE.lock().unwrap();
        (state.config.clone().unwrap_or_default(), state.paused)
    };
    unsafe {
        msg_bool(menu, sel("setAutoenablesItems:"), false);
        add_status(menu, &status_line());
        msg_void_id(menu, sel("addItem:"), separator());
        add_action(
            menu,
            target,
            &format!("Reflow: {}", config.kind_name()),
            "reflow:",
        );
        add_action(
            menu,
            target,
            if paused { "Resume" } else { "Pause" },
            "togglePause:",
        );
        msg_void_id(menu, sel("addItem:"), separator());
        add_status(menu, &settings_line());
        add_action(menu, target, "Settings\u{2026}", "showSettings:");
        add_action(menu, target, "Reset to defaults", "resetDefaults:");
        msg_void_id(menu, sel("addItem:"), separator());
        add_status(menu, &accessibility_line());
        add_action(menu, target, "Diagnostics", "diagnostics:");
        msg_void_id(menu, sel("addItem:"), separator());
        add_action(menu, target, "Quit", "quit:");
    }
    menu
}

/// Replace the status item's menu with a fresh one.
fn refresh_menu() {
    let (item, target) = {
        let state = STATE.lock().unwrap();
        (state.status_item, state.target)
    };
    if item.is_null() || target.is_null() {
        return;
    }
    // SAFETY: `item` is a live NSStatusItem and `target` a live object.
    let menu = unsafe { build_menu(target.0) };
    unsafe { msg_void_id(item.0, sel("setMenu:"), menu) };
}

// MARK: - Shell logic

/// One-line status: paused state, Accessibility trust, fixture, last result.
fn status_line() -> String {
    let state = STATE.lock().unwrap();
    status_line_with(&state)
}

/// The status line from an already-locked state: callers that hold `STATE`
/// (diagnostics) use this so the mutex is never re-locked.
fn status_line_with(state: &ShellState) -> String {
    let trust = match MacAxAdapter::connect() {
        Ok(_) => "Accessibility: granted",
        Err(_) => "Accessibility: denied",
    };
    let fixture = match fixture::registered_fixture_pid() {
        Some(pid) if process_alive(pid) => format!("fixture pid={pid}"),
        _ => "no live fixture".to_string(),
    };
    let pause = if state.paused { "paused" } else { "active" };
    let status = if state.status.is_empty() {
        String::new()
    } else {
        format!(" · {}", state.status)
    };
    format!("{pause} · {trust} · {fixture}{status}")
}

/// Accessibility-only line for the menu.
fn accessibility_line() -> String {
    match MacAxAdapter::connect() {
        Ok(_) => "Accessibility: granted".to_string(),
        Err(_) => {
            "Accessibility: denied — grant it in System Settings > Privacy & Security".to_string()
        }
    }
}

/// Settings summary line for the menu.
fn settings_line() -> String {
    let state = STATE.lock().unwrap();
    let config = state.config.clone().unwrap_or_default();
    settings_line_with(&config)
}

/// The settings summary from a config (no re-lock; diagnostics calls this).
fn settings_line_with(config: &Config) -> String {
    let auto_tiling = if config.auto_tiling { "on" } else { "off" };
    let dialogs = if config.include_dialogs { "in" } else { "out" };
    format!(
        "settings: layout={} gaps={}/{} ratio={} auto-tiling={auto_tiling} dialogs={dialogs} rules={}",
        config.kind_name(),
        fmt_num(config.layout.outer_gap),
        fmt_num(config.layout.inner_gap),
        fmt_num(config.layout.master_ratio),
        config.rules.len(),
    )
}

/// Render an f64 without trailing noise.
fn fmt_num(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Append a line to the shell log. Best-effort: a log failure is never fatal
/// and never affects window state.
pub(crate) fn append_log(line: &str) {
    use std::io::Write;
    if let Some(home) = std::env::var_os("HOME") {
        let path = std::path::Path::new(&home)
            .join(".hypr-rust")
            .join("shell.log");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
    eprintln!("{line}");
}

/// Usable work area: the main display bounds with the system menu bar removed
/// from the top. AX frames share the same top-left-origin global space.
fn work_area() -> Rect {
    // SAFETY: both are documented CoreGraphics entry points with no state.
    let bounds = unsafe { ffi::CGDisplayBounds(ffi::CGMainDisplayID()) };
    let inset = ffi::K_MENU_BAR_INSET;
    Rect::new(
        bounds.origin.x,
        bounds.origin.y + inset,
        bounds.size.width.max(1.0),
        (bounds.size.height - inset).max(1.0),
    )
}

/// Main display bounds **including** the menu-bar strip: the membership
/// space for "this window lives on the main display". [`work_area`] is the
/// same rectangle minus that strip.
fn display_bounds() -> Rect {
    // SAFETY: both are documented CoreGraphics entry points with no state.
    let bounds = unsafe { ffi::CGDisplayBounds(ffi::CGMainDisplayID()) };
    Rect::new(
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.width.max(1.0),
        bounds.size.height.max(1.0),
    )
}

/// Per-window rule overrides for one reflow: the app's first matching rule
/// decides the policy for that app (ADR-0006).
///
/// - `DialogPolicy::Exclude` → an `exclude` override for **every** window of
///   that app — app protection: its normal windows are protected too, not
///   only its dialogs.
/// - `DialogPolicy::Include` → an `include` override (widening the dialog
///   policy); the planner consults it only for `Dialog` windows.
/// - `DialogPolicy::Default` → no overrides; the global `include_dialogs`
///   policy in the planner applies unchanged.
///
/// `windows` are the planner IDs of that app's windows in the observation
/// (local keys for the fixture path, global keys for the merged path).
/// Mandatory roles never appear as effective overrides (planner gates keep
/// them excluded regardless; rules cannot unlock mandatory).
///
/// SAFETY: main thread; AppKit query only (bundle id, never titles).
fn window_rule_overrides(
    pid: u32,
    windows: &[layout_core::WindowId],
    rules: &[WindowRule],
) -> Vec<layout_core::WindowRuleOverride> {
    let bundle = unsafe { app_bundle_id(pid) };
    let (exclude, include) = match dialog_policy_for(rules, pid, bundle.as_deref()) {
        DialogPolicy::Exclude => (true, None),
        DialogPolicy::Include => (false, Some(true)),
        // No override: the planner follows the global policy per window.
        DialogPolicy::Default => return Vec::new(),
    };
    windows
        .iter()
        .map(|window| layout_core::WindowRuleOverride {
            window: *window,
            exclude,
            include,
        })
        .collect()
}

/// Run one explicit reflow against the registered fixture.
///
/// The layout, gaps and dialog policy come from the loaded config; only an
/// explicit user command reaches the batch (best-effort rollback,
/// [ADR-0005](../adr/0005-batch-multiwindow-best-effort-rollback.md)).
fn perform_reflow() {
    if STATE.lock().unwrap().paused {
        set_status("paused");
        return;
    }
    let (kind, layout, include_dialogs, label, rules) = {
        let config = current_config();
        let (kind, layout) = config.plan_config();
        (
            kind,
            layout,
            config.include_dialogs,
            config.kind_name().to_string(),
            config.rules.clone(),
        )
    };
    let adapter = match MacAxAdapter::connect() {
        Ok(adapter) => adapter,
        Err(error) => {
            set_status(&format!("{label}: {error}"));
            return;
        }
    };
    let pid = match fixture::registered_fixture_pid() {
        Some(pid) if process_alive(pid) => pid,
        _ => {
            set_status(&format!("{label}: no live fixture"));
            return;
        }
    };
    let set = match adapter.windows(pid) {
        Ok(set) => set,
        Err(error) => {
            set_status(&format!("{label}: {error}"));
            return;
        }
    };
    let observation = set.observe(work_area());
    let window_ids: Vec<layout_core::WindowId> = set
        .entries()
        .map(|(key, _, _)| layout_core::WindowId(key.0))
        .collect();
    let window_rules = window_rule_overrides(pid, &window_ids, &rules);
    let plan = match layout_core::plan_snapshot_with_rules(
        &observation.snapshot,
        DisplayId(0),
        kind,
        layout,
        include_dialogs,
        &window_rules,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            set_status(&format!("{label}: plan {error:?}"));
            return;
        }
    };
    let targets = match batch::targets_for_plan(&plan, &observation.map) {
        Ok(mut targets) => {
            // macOS enforces a minimum window height (AX exposes no min-size
            // attribute, so the floor is host-measured): a plan that emits a
            // shorter row would be clamped by the system, fail the strict
            // read-back (ADR-0006), and roll back — making the layout look
            // inert. Snap short rows up to the floor before the write.
            batch::snap_short_rows(&mut targets);
            targets
        }
        Err(error) => {
            set_status(&format!("{label}: map {error:?}"));
            return;
        }
    };
    let result = batch::apply_batch(&set, &targets);
    let excluded = plan.excluded.len();
    let status = if let Some(error) = &result.error {
        format!(
            "{label}: failed at key {:?} ({error})",
            result.failed_at.map(|key| key.0)
        )
    } else {
        format!(
            "{label}: applied {} window(s){}",
            result.outcomes.len(),
            if excluded > 0 {
                format!(", {excluded} excluded")
            } else {
                String::new()
            }
        )
    };
    set_status(&status);
}

/// The automatic action of event-driven auto-tiling (ADR-0008 §3): the
/// **same** strict command the menu click and the ⌥⌘R hotkey run, called
/// from the coalescing timer on the main run loop. While paused the event
/// is dropped silently — no re-tile, no status churn (the chip already
/// says the shell is paused).
pub(crate) fn perform_reflow_automatic() {
    if STATE.lock().unwrap().paused {
        // Trace point 3 of the automatic path (diagnostic): the tick
        // arrived but the shell is paused, so the event is dropped.
        append_log("auto-tiling: tick dropped (paused)");
        return;
    }
    append_log("auto-tiling: automatic reflow");
    perform_reflow_real();
}

/// Run one explicit reflow over **real app windows** (ADR-0006 scope (a)):
/// every normal window of every regular app on the main display, applied
/// with the strict no-partial batch.
///
/// The layout, gaps, dialog policy and rules come from the loaded config;
/// only an explicit user command or an owner-enabled automatic event
/// ([ADR-0008](../../docs/adr/0008-auto-tiling-event-driven.md) — same
/// function, no second path) reaches the batch. Automated evidence
/// (`--self-test`, `--panel-click-proof`) never calls this function — those
/// stay bound to the registered fixture via [`perform_reflow`].
///
/// Window titles are never read: only AX geometry, roles, minimized flags,
/// bundle ids and process identifiers.
fn perform_reflow_real() {
    if STATE.lock().unwrap().paused {
        set_status("paused");
        return;
    }
    let (kind, layout, include_dialogs, label, rules) = {
        let config = current_config();
        let (kind, layout) = config.plan_config();
        (
            kind,
            layout,
            config.include_dialogs,
            config.kind_name().to_string(),
            config.rules.clone(),
        )
    };
    let adapter = match MacAxAdapter::connect() {
        Ok(adapter) => adapter,
        Err(error) => {
            set_status(&format!("{label}: {error}"));
            return;
        }
    };

    // Preflight - enumerate every regular app before the first write
    // (ADR-0006 §2.1, amended 2026-10-27). `NoWindows` means "this app has
    // no targets" and is skipped; any other enumeration error is retried
    // once and, if it persists, the app is skipped and named in the status
    // (`unavailable skipped: pid …`) — a skip writes nothing. Target-level
    // read failures still refuse with zero writes inside the strict batch.
    let work = work_area();
    let display = display_bounds();
    let mut merged = MacMultiWindowSet::new();
    let mut window_rules: Vec<layout_core::WindowRuleOverride> = Vec::new();
    let mut skipped = 0usize;
    let mut unavailable: Vec<u32> = Vec::new();
    for pid in regular_app_pids() {
        // ADR-0006 §2.1 (amended 2026-10-27): retry an app-level
        // enumeration failure once — a transient `-25204 cannotComplete`
        // from a busy app must not veto the whole command — then skip the
        // app and name it in the status. Skipping writes nothing: targets
        // are built only from windows that were read successfully, and
        // target-level read/write failures stay strict below.
        let mut set = match adapter.windows(pid) {
            Ok(set) => set,
            Err(AxError::NoWindows) => continue,
            Err(first) => match adapter.windows(pid) {
                Ok(set) => set,
                Err(_) => {
                    append_log(&format!(
                        "{label}: app pid={pid} unavailable after retry ({first})"
                    ));
                    unavailable.push(pid);
                    continue;
                }
            },
        };
        skipped += set.drop_non_target_windows(work, display);
        if set.is_empty() {
            continue;
        }
        // The merged observation re-keys to global ids (pid << 32 | index);
        // the overrides must name those same ids.
        let window_ids: Vec<layout_core::WindowId> = set
            .entries()
            .map(|(key, _, _)| layout_core::WindowId(global_window_key(pid, key.0).0))
            .collect();
        window_rules.extend(window_rule_overrides(pid, &window_ids, &rules));
        merged.push(set);
    }
    if merged.is_empty() {
        let mut status = format!("{label}: no target windows");
        push_unavailable(&mut status, &unavailable);
        append_log(&status);
        set_status(&status);
        return;
    }
    // Master (ADR-0006 §4): the window focused at command time — the
    // frontmost app's focused window, if that app is part of this batch.
    if let Some(front) = frontmost_pid() {
        merged.set_focused_pid(front);
    }

    let observation = merged.observe(work);
    let plan = match layout_core::plan_snapshot_with_rules(
        &observation.snapshot,
        DisplayId(0),
        kind,
        layout,
        include_dialogs,
        &window_rules,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            let status = format!("{label}: plan {error:?} (0 windows touched)");
            append_log(&status);
            set_status(&status);
            return;
        }
    };
    let targets = match batch::targets_for_plan(&plan, &observation.map) {
        Ok(mut targets) => {
            // macOS enforces a minimum window height (AX exposes no min-size
            // attribute, so the floor is host-measured): a plan that emits a
            // shorter row would be clamped by the system, fail the strict
            // read-back (ADR-0006), and roll the whole batch back — making the
            // layout look inert. Snap short rows up to the floor before the
            // write; `snap_short_rows` leaves the plan untouched when snapping
            // would collide adjacent frames.
            batch::snap_short_rows(&mut targets);
            targets
        }
        Err(error) => {
            let status = format!("{label}: map {error:?} (0 windows touched)");
            append_log(&status);
            set_status(&status);
            return;
        }
    };
    let result = batch::apply_batch_strict(&merged, &targets);
    let mut status = strict_status_text(&label, &result, plan.excluded.len(), skipped);
    push_unavailable(&mut status, &unavailable);
    append_log(&status);
    set_status(&status);
}

/// Name the apps the enumeration skipped after the §2.1 amendment's retry
/// (ADR-0006): pids only — never a window title.
fn push_unavailable(status: &mut String, unavailable: &[u32]) {
    if unavailable.is_empty() {
        return;
    }
    status.push_str(&format!(
        ", unavailable skipped: pid {}",
        unavailable
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ));
}

/// Name the batch's failing window for a status line: an app pid plus its
/// local window index for a global key (`pid << 32 | index`), or the raw key
/// otherwise. Pids only — never a window title (ADR-0006 §2).
fn failed_at_note(result: &batch::StrictBatchResult) -> String {
    match result.result.failed_at {
        Some(key) => {
            let (pid, local) = split_global_window_key(key);
            if pid == 0 {
                format!("; failed at key {}", key.0)
            } else {
                format!("; failed at app pid {pid} (window {local})")
            }
        }
        None => String::new(),
    }
}

/// The ADR-0006 three-way status line: success / clean refusal / loud
/// partial state. A partial state names the observed frames, is never
/// phrased as success, and the word "atomic" is never used.
fn strict_status_text(
    label: &str,
    result: &batch::StrictBatchResult,
    excluded: usize,
    skipped: usize,
) -> String {
    let mut extras = String::new();
    if excluded > 0 {
        extras.push_str(&format!(", {excluded} excluded"));
    }
    if skipped > 0 {
        extras.push_str(&format!(
            ", {skipped} skipped (minimized/fullscreen/off-display)"
        ));
    }
    let failed = failed_at_note(result);
    let error_of = |r: &batch::StrictBatchResult| {
        r.result
            .error
            .as_ref()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    };
    match result.status() {
        batch::StrictStatus::Success => format!(
            "{label}: tiled {} window(s){extras}",
            result.result.outcomes.len()
        ),
        batch::StrictStatus::CleanRefusal if result.refused_before_write => {
            format!(
                "{label}: refused before write ({}) — 0 windows touched{failed}{extras}",
                error_of(result)
            )
        }
        batch::StrictStatus::CleanRefusal => format!(
            "{label}: refused ({}) — all windows restored{failed}{extras}",
            error_of(result)
        ),
        batch::StrictStatus::LoudPartial => {
            let frames: Vec<String> = result
                .result
                .outcomes
                .iter()
                .filter_map(|(key, outcome)| match outcome {
                    batch::WindowOutcome::RolledBack { restored, actual } => Some(format!(
                        "key {}: restored {restored:?}, observed {}",
                        key.0,
                        actual
                            .map(|frame| format!("{frame:?}"))
                            .unwrap_or_else(|| "unreadable".to_string())
                    )),
                    batch::WindowOutcome::Applied { .. } => None,
                })
                .collect();
            format!(
                "{label}: PARTIAL STATE ({}) — restore unverified: {}",
                error_of(result),
                frames.join("; ")
            )
        }
    }
}

fn set_status(status: &str) {
    STATE.lock().unwrap().status = status.to_string();
    refresh_menu();
}

// MARK: - Entry point

/// Cast an `extern "C" fn(Id, Sel)` implementation to an Objective-C `IMP`.
#[allow(clippy::fn_to_numeric_cast)]
fn imp(f: extern "C" fn(Id, Sel)) -> *const std::os::raw::c_void {
    f as usize as *const std::os::raw::c_void
}

/// Cast an `extern "C" fn(Id, Sel, Id)` implementation — an action that
/// receives one object argument (the sending control) — to an IMP.
#[allow(clippy::fn_to_numeric_cast)]
fn imp3(f: extern "C" fn(Id, Sel, Id)) -> *const std::os::raw::c_void {
    f as usize as *const std::os::raw::c_void
}

/// Register the action-target class and return one instance.
///
/// SAFETY: each `class_addMethod` pairs a registered selector with an
/// implementation matching its encoding: `v@:` for the two-argument actions
/// and `v@:@` for `removeRule:`, which takes the sending control as its
/// third argument.
unsafe fn create_target() -> Id {
    let superclass = class("NSObject");
    let name = b"HyprRustShell\0";
    // SAFETY: NSObject is a live class and `name` is a NUL-terminated C string.
    let cls = unsafe { ffi::objc_allocateClassPair(superclass, name.as_ptr() as *const c_char, 0) };
    assert!(!cls.is_null(), "failed to allocate the HyprRustShell class");
    let types = b"v@:\0";
    let sender_types = b"v@:@\0";
    // SAFETY: the selectors are registered above and the implementations match
    // the `v@:` encoding.
    unsafe {
        ffi::class_addMethod(
            cls,
            sel("reflow:"),
            imp(action_reflow),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("togglePause:"),
            imp(action_toggle_pause),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("showSettings:"),
            imp(action_show_settings),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("saveSettings:"),
            imp(action_save_settings),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("closeSettings:"),
            imp(action_close_settings),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("resetDefaults:"),
            imp(action_reset_defaults),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("diagnostics:"),
            imp(action_diagnostics),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("quit:"),
            imp(action_quit),
            types.as_ptr() as *const c_char,
        );
        // Per-row rules actions: `addRule:` takes no extra argument;
        // `removeRule:` receives the row's remove button as the sender.
        ffi::class_addMethod(
            cls,
            sel("addRule:"),
            imp(action_add_rule),
            types.as_ptr() as *const c_char,
        );
        ffi::class_addMethod(
            cls,
            sel("removeRule:"),
            imp3(action_remove_rule),
            sender_types.as_ptr() as *const c_char,
        );
        // ADR-0008 §2: NSWorkspace app launch/terminate notifications carry
        // the posting NSNotification as the single object argument
        // (`v@:@`).
        ffi::class_addMethod(
            cls,
            sel("workspaceAppEvent:"),
            imp3(action_workspace_app_event),
            sender_types.as_ptr() as *const c_char,
        );
        ffi::objc_registerClassPair(cls);
    }
    // SAFETY: `cls` is a registered class.
    unsafe { ffi::class_createInstance(cls, 0) }
}

/// Run the shell: create the app, status item and menu, then enter the run loop.
///
/// Objects created here live for the whole session (the status item, its menu
/// and the action target), so they are intentionally not released.
pub fn run() {
    unsafe {
        let app = msg0(class("NSApplication"), sel("sharedApplication"));
        assert!(!app.is_null(), "NSApplication unavailable");
        msg_int(
            app,
            sel("setActivationPolicy:"),
            ffi::NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY,
        );
        msg_bool(app, sel("activateIgnoringOtherApps:"), true);

        let target = create_target();
        assert!(!target.is_null(), "failed to create the action target");

        // Load the config before building the menu so the menu reflects it.
        STATE.lock().unwrap().config = Some(load_config_or_default());

        // ADR-0007: global hotkey ⌥⌘R — the same action as the menu's
        // reflow (`perform_reflow_real`), registered once here in `run()`
        // only; the fixture-bound automated paths never reach this line.
        // Fail-soft: a chord conflict logs and leaves the menu as the
        // working surface.
        match crate::hotkey::register(perform_reflow_real) {
            Ok(()) => append_log("hotkey: registered ⌥⌘R (reflow)"),
            Err(error) => append_log(&format!(
                "hotkey: not registered — {error}; menu reflow still works"
            )),
        }

        let bar = msg0(class("NSStatusBar"), sel("systemStatusBar"));
        let item = msg_cgfloat(
            bar,
            sel("statusItemWithLength:"),
            ffi::NS_VARIABLE_STATUS_ITEM_LENGTH,
        );
        msg_void_id(item, sel("setMenu:"), build_menu(target));
        let button = msg0(item, sel("button"));
        msg_void_id(button, sel("setTitle:"), nsstring("\u{25e7}"));

        {
            let mut state = STATE.lock().unwrap();
            state.app = Handle(app);
            state.status_item = Handle(item);
            state.target = Handle(target);
        }
        // ADR-0008 §2: observe app launch/terminate for the auto-tiling
        // observers (the handler is a no-op while the toggle is off). The
        // notification names are the exported AppKit constants.
        let workspace = msg0(class("NSWorkspace"), sel("sharedWorkspace"));
        let center = msg0(workspace, sel("notificationCenter"));
        msg_id_id_id_id(
            center,
            sel("addObserver:selector:name:object:"),
            target,
            sel("workspaceAppEvent:"),
            ffi::NSWorkspaceDidLaunchApplicationNotification as Id,
            std::ptr::null(),
        );
        msg_id_id_id_id(
            center,
            sel("addObserver:selector:name:object:"),
            target,
            sel("workspaceAppEvent:"),
            ffi::NSWorkspaceDidTerminateApplicationNotification as Id,
            std::ptr::null(),
        );
        // ADR-0008 §1: honor the toggle from the previous session (attach
        // is best-effort; failures are logged once per app, §4).
        let auto_tiling_on = STATE
            .lock()
            .unwrap()
            .config
            .as_ref()
            .map(|config| config.auto_tiling)
            .unwrap_or(false);
        if auto_tiling_on {
            crate::auto_tiling::set_enabled(true);
        }
        msg_void(app, sel("run"));
    }
}

/// Read the titles of a menu's items, in order.
///
/// SAFETY: `menu` must be a live NSMenu.
unsafe fn menu_titles(menu: Id) -> Vec<String> {
    let count = unsafe { msg_isize(menu, sel("numberOfItems")) };
    let mut titles = Vec::new();
    for index in 0..count {
        let item = unsafe { msg_id(menu, sel("itemAtIndex:"), index as Id) };
        let title = unsafe { msg0(item, sel("title")) };
        let text = unsafe { msg_cstr(title, sel("UTF8String")) };
        if !text.is_null() {
            let text = unsafe { std::ffi::CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned();
            titles.push(text);
        }
    }
    titles
}

/// Exercise the shell without a run loop: create the app, status item and menu,
/// report the menu contents, then run one reflow. Used by
/// `ax-shell --self-test` as host evidence without a mouse click.
///
/// Objects created here live for the process, which exits right after.
/// Exit codes of the fixture-bound self-test.
/// 0 - full pass: the pre-run config was restored, or an absent pre-run file
///     was removed again.
/// 1 - restore (or no-file removal) failed; the run must not look successful.
/// 2 - skipped run: the pre-run config was invalid/unsafe, so no test that
///     writes or resets the config ever started; the file is left untouched.
const K_SELF_TEST_EXIT_OK: i32 = 0;
const K_SELF_TEST_EXIT_RESTORE_FAILED: i32 = 1;
const K_SELF_TEST_EXIT_SKIPPED: i32 = 2;

/// One classification of the config file state before a self-test may write.
enum PreRun {
    /// The file loads through `Config::load`, which enforces owner-only 0600.
    Valid(Config),
    /// No config file exists.
    Missing,
    /// Malformed content, or a file this app would never itself write
    /// (symlink, foreign uid, mode other than 0600). The run must not touch it.
    Unsafe(String),
}

fn classify_pre_run_at(
    path: &std::path::Path,
    load_config: impl FnOnce() -> Result<Config, ConfigError>,
) -> PreRun {
    // `metadata` follows a final symlink. A dangling symlink therefore reports
    // NotFound and could be mistaken for an absent config, allowing the test's
    // save/cleanup path to replace and delete the user's symlink. Inspect the
    // directory entry itself so every symlink is rejected before any writes.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        // Missing is a first-class state: the run may write and must restore
        // to no-file at the end.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return PreRun::Missing,
        Err(error) => return PreRun::Unsafe(format!("unreadable: {error}")),
    };
    if meta.file_type().is_symlink() {
        return PreRun::Unsafe("config is a symlink".to_string());
    }
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    match load_config() {
        Ok(config) => PreRun::Valid(config),
        Err(crate::config::ConfigError::NotOwnerOnly) => {
            PreRun::Unsafe(format!("not owner-only (mode {mode:o}) or a foreign uid"))
        }
        Err(error) => PreRun::Unsafe(format!("config invalid: {error}")),
    }
}

fn classify_pre_run() -> PreRun {
    let path = match crate::config::config_path() {
        Ok(path) => path,
        Err(error) => return PreRun::Unsafe(format!("home path unavailable: {error}")),
    };
    classify_pre_run_at(&path, Config::load)
}

pub fn self_test() -> i32 {
    // ADR-0008 §5: automated evidence never attaches auto-tiling observers,
    // whatever the config says — set_enabled is a hard no-op in this mode.
    crate::auto_tiling::set_automated(true);
    unsafe {
        // The pre-run file is classified before anything in this run may
        // write: valid, missing, or invalid/unsafe (fail closed, untouched).
        let pre_run = classify_pre_run();

        // AppKit requires an autorelease pool on the thread that creates
        // autoreleased objects.
        let pool = msg0(class("NSAutoreleasePool"), sel("new"));
        let app = msg0(class("NSApplication"), sel("sharedApplication"));
        assert!(!app.is_null(), "NSApplication unavailable");
        msg_int(
            app,
            sel("setActivationPolicy:"),
            ffi::NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY,
        );

        let target = create_target();
        assert!(!target.is_null(), "failed to create the action target");

        // Load the config before building the menu so the menu reflects it.
        STATE.lock().unwrap().config = Some(load_config_or_default());

        let menu = build_menu(target);
        let titles = menu_titles(menu);

        let bar = msg0(class("NSStatusBar"), sel("systemStatusBar"));
        let item = msg_cgfloat(
            bar,
            sel("statusItemWithLength:"),
            ffi::NS_VARIABLE_STATUS_ITEM_LENGTH,
        );
        msg_void_id(item, sel("setMenu:"), menu);
        {
            let mut state = STATE.lock().unwrap();
            state.app = Handle(app);
            state.status_item = Handle(item);
            state.target = Handle(target);
        }

        append_log(&format!("self-test: menu items = {titles:?}"));
        append_log(&format!("self-test: status = {}", status_line()));
        perform_reflow();
        append_log(&format!("self-test: after reflow = {}", status_line()));

        // The classification subsumes every config write below. The reflow
        // just performed used only the in-memory state - no disk writes are
        // triggered by it.
        if let PreRun::Unsafe(reason) = &pre_run {
            append_log(&format!(
                "self-test: pre-run config classified invalid/unsafe ({reason}); config-mutating sections not started, file left untouched"
            ));
            action_diagnostics(std::ptr::null(), std::ptr::null());
            append_log(&format!(
                "self-test: exit {K_SELF_TEST_EXIT_SKIPPED} (skipped run)"
            ));
            msg_void(pool, sel("drain"));
            return K_SELF_TEST_EXIT_SKIPPED;
        }
        match &pre_run {
            PreRun::Valid(_) => {
                append_log("self-test: pre-run config classified valid (owner-only 0600); full run")
            }
            PreRun::Missing => append_log("self-test: pre-run config classified missing; full run"),
            PreRun::Unsafe(_) => unreachable!("handled above"),
        }

        // Rules: the first matching rule shifts the dialog policy for that
        // app only. Rule exclude wins over a global include, then rule include
        // wins over a global exclude; both end with a reset so the panel
        // exercise starts from plain defaults.
        if let Some(pid) = fixture::registered_fixture_pid().filter(|pid| process_alive(*pid)) {
            let bundle = app_bundle_id(pid);
            append_log(&format!("self-test: fixture bundle id = {bundle:?}"));

            // Rule exclude with global include: the dialog is dropped.
            let mut config = Config {
                include_dialogs: true,
                ..Config::default()
            };
            config.rules.push(WindowRule {
                bundle: None,
                pid: Some(pid),
                dialogs: DialogPolicy::Exclude,
            });
            // A failed write must not abort the run before the restore can
            // report; log and continue (the final exit code governs success).
            if let Err(error) = config.save() {
                append_log(&format!("self-test: rules config write failed: {error}"));
            }
            STATE.lock().unwrap().config = Some(config);
            perform_reflow();
            append_log(&format!(
                "self-test: rule exclude + dialogs=on = {}",
                status_line()
            ));

            // Rule include with global exclude: the dialog is tiled.
            let mut config = Config::default();
            config.rules.push(WindowRule {
                bundle: None,
                pid: Some(pid),
                dialogs: DialogPolicy::Include,
            });
            // A failed write must not abort the run before the restore can
            // report; log and continue (the final exit code governs success).
            if let Err(error) = config.save() {
                append_log(&format!("self-test: rules config write failed: {error}"));
            }
            STATE.lock().unwrap().config = Some(config);
            perform_reflow();
            append_log(&format!(
                "self-test: rule include + dialogs=off = {}",
                status_line()
            ));

            action_reset_defaults(std::ptr::null(), std::ptr::null());
            STATE.lock().unwrap().config = Some(Config::load().unwrap_or_default());
            append_log(&format!(
                "self-test: defaults restored after rules = {}",
                status_line()
            ));
        } else {
            append_log("self-test: no live fixture — rules exercise skipped");
        }

        // Settings panel + per-row rules editor: open through the same path
        // the menu action uses, read the controls and the rule rows back,
        // save through the Save button's own helper, prove invalid input is
        // refused, then restore defaults for the next run. A seeded rule is
        // in place before the panel opens, so the rows display it and a save
        // round-trips it.
        let mut seeded = Config::load().unwrap_or_default();
        seeded.rules.push(WindowRule {
            bundle: Some("com.example.seeded".to_string()),
            pid: None,
            dialogs: DialogPolicy::Exclude,
        });
        STATE.lock().unwrap().config = Some(seeded);
        show_settings();
        {
            let panel = STATE.lock().unwrap().settings.expect("panel built");
            // SAFETY: within self_test's enclosing unsafe block; live handles.
            let (popup, outer, inner, ratio) = (
                popup_title(panel.popup),
                field_text(panel.outer),
                field_text(panel.inner),
                field_text(panel.ratio),
            );
            let (auto, dialogs) = (checkbox_on(panel.auto_tiling), checkbox_on(panel.dialogs));
            // The rows are the editor surface: log the row count and what the
            // first row shows (the entry text for a bundle row, the read-only
            // pid label for a pid row) plus its policy title.
            let (rows_count, first_row) = {
                let rows = RULE_ROWS.lock().unwrap();
                let count = rows.len();
                // SAFETY: within self_test's enclosing unsafe block; live
                // row handles.
                let first = rows.first().map(|row| {
                    if let Some(pid) = row.pid {
                        format!("pid {pid} (from file) policy={:?}", popup_title(row.policy))
                    } else {
                        format!(
                            "bundle entry={:?} policy={:?}",
                            field_text(row.entry),
                            popup_title(row.policy)
                        )
                    }
                });
                (count, first)
            };
            let visible = msg_u8(panel.window.0, sel("isVisible"));
            append_log(&format!(
                "self-test: panel visible={visible} popup={popup:?} outer={outer} inner={inner} ratio={ratio} auto={auto} dialogs={dialogs} rows={rows_count} first-row={first_row:?}"
            ));
            // 1) A valid custom value saved through the Save button's code
            // path; the seeded rule round-trips through the editor.
            select_popup_title(panel.popup, "columns");
            set_field_text(panel.outer, "20");
            set_field_text(panel.inner, "4");
            set_field_text(panel.ratio, "0.7");
            set_checkbox(panel.dialogs, true);
            save_settings_from_panel();
            append_log(&format!(
                "self-test: custom save status = {}",
                STATE.lock().unwrap().status.clone()
            ));
            // Custom values and the seeded rule must have landed on disk
            // through Config::save.
            append_log(&format!(
                "self-test: loaded after custom save = {:?}",
                Config::load()
            ));
            // 2) An invalid value is refused; the file stays untouched.
            set_field_text(panel.ratio, "5");
            save_settings_from_panel();
            append_log(&format!(
                "self-test: invalid save status = {}",
                STATE.lock().unwrap().status.clone()
            ));
            // 3) Rules edited at the row level: the seeded bundle row is
            // replaced by a pid row (fixture-bound; the read-only label
            // round-trips the pid) or a bundle row typed at the row surface;
            // both with the include dialog policy.
            set_field_text(panel.ratio, "0.7");
            let target = STATE.lock().unwrap().target.0;
            rebuild_rules_rows(panel.rows_area, &[], target);
            let round3 = if let Some(pid) =
                fixture::registered_fixture_pid().filter(|pid| process_alive(*pid))
            {
                add_pid_rule_row(pid);
                format!("pid {pid} include")
            } else {
                if add_rule_row() {
                    let row = RULE_ROWS.lock().unwrap()[0];
                    set_field_text(row.entry, "com.example.edited");
                    select_popup_title(row.policy, "include");
                } else {
                    append_log("self-test: add-rule row unavailable (row edit skipped)");
                }
                "com.example.edited include".to_string()
            };
            save_settings_from_panel();
            append_log(&format!(
                "self-test: rules edit ({round3}) save status = {}",
                STATE.lock().unwrap().status.clone()
            ));
            append_log(&format!(
                "self-test: loaded after rules edit = {:?}",
                Config::load()
            ));
            // 4) A matcher-less row is refused: the added empty bundle row has
            // neither bundle nor pid, and the final from_json validation (the
            // same entry validation a file load runs) rejects it; the file
            // keeps the last saved rules list.
            add_rule_row();
            save_settings_from_panel();
            append_log(&format!(
                "self-test: matcher-less save status = {}",
                STATE.lock().unwrap().status.clone()
            ));
            append_log(&format!(
                "self-test: loaded after matcher-less = {:?}",
                Config::load().map(|config| config.rules)
            ));
            // 5) Final exercise of the reset action's own code path.
            action_reset_defaults(std::ptr::null(), std::ptr::null());
            append_log(&format!(
                "self-test: final settings status = {}",
                STATE.lock().unwrap().status.clone()
            ));
        }
        // 6) Restore exactly what the run started with, so the run is
        // transparent to the profile: the file through the app's own save
        // path and the in-memory state adopted, and a pre-run state with no
        // file returns to no file (the file this run created is removed). A
        // restore that fails fails the run - never looks successful.
        let exit_code = match pre_run {
            PreRun::Valid(config) => match config.save() {
                Ok(()) => {
                    STATE.lock().unwrap().config = Some(config);
                    set_status("config restored to its pre-run state");
                    append_log("self-test: config restored to its pre-run state");
                    K_SELF_TEST_EXIT_OK
                }
                Err(error) => {
                    append_log(&format!("self-test: config restore failed: {error}"));
                    K_SELF_TEST_EXIT_RESTORE_FAILED
                }
            },
            PreRun::Missing => match crate::config::config_path() {
                Ok(path) => match std::fs::remove_file(&path) {
                    Ok(()) => {
                        set_status("pre-run config was absent; run-created file removed");
                        append_log(
                            "self-test: pre-run config was absent; removed the file this run created",
                        );
                        K_SELF_TEST_EXIT_OK
                    }
                    Err(error) => {
                        append_log(&format!(
                            "self-test: restoring the no-file state failed: {error}"
                        ));
                        K_SELF_TEST_EXIT_RESTORE_FAILED
                    }
                },
                Err(error) => {
                    append_log(&format!(
                        "self-test: home path for restore unavailable: {error}"
                    ));
                    K_SELF_TEST_EXIT_RESTORE_FAILED
                }
            },
            // The skipped run returned above.
            PreRun::Unsafe(_) => K_SELF_TEST_EXIT_RESTORE_FAILED,
        };
        // 7) Diagnostics after restore: must match what is (or is not) on
        // disk - the summary reads the adopted in-memory state.
        action_diagnostics(std::ptr::null(), std::ptr::null());
        append_log(&format!("self-test: exit {exit_code}"));
        msg_void(pool, sel("drain"));
        exit_code
    }
}

/// Click-path proof for the settings panel (docs/ARENA-VERIFICATION Path L).
///
/// The self-test calls `save_settings_from_panel()` directly — a Rust function
/// call. This proof instead dispatches Save through the real button action:
/// `performClick:` is the AppKit dispatch an accessibility press and a mouse
/// click land on. No synthetic mouse events are used and the event-delivery
/// layer is not exercised; a strictly manual variant (the owner's hand on the
/// mouse) remains possible on request. `performClick:` is synchronous: the
/// whole Save path has run by the time the call returns.
///
/// Protective guard: a pre-existing config file is a real owner profile — the
/// proof skips fail-closed (nothing read, nothing written). A run that starts
/// without a file may create one through Save and removes it again at the end
/// (returns to no-file, the same end-state decision as the self-test); a
/// failed removal fails the proof rather than looking successful.
///
/// The app is bootstrapped exactly as the menu app is (NSApplication, action
/// target, menu, status item), so the button's action dispatch runs inside a
/// normally wired app, not a stripped test rig.
///
/// Exit codes (same semantics as the self-test): 0 pass, 1 proof failure,
/// 2 skipped.
pub fn panel_click_proof() -> i32 {
    // ADR-0008 §5: automated evidence never attaches auto-tiling observers,
    // whatever the config says — set_enabled is a hard no-op in this mode.
    crate::auto_tiling::set_automated(true);
    let path = match crate::config::config_path() {
        Ok(path) => path,
        Err(error) => {
            append_log(&format!(
                "panel click proof: config path unavailable: {error}"
            ));
            return K_SELF_TEST_EXIT_RESTORE_FAILED;
        }
    };
    if path.exists() {
        append_log(
            "panel click proof: skipped (config file pre-exists; refusing to touch a real profile)",
        );
        return K_SELF_TEST_EXIT_SKIPPED;
    }
    unsafe {
        // AppKit requires an autorelease pool on the thread that creates
        // autoreleased objects (same bootstrap as self_test / run()).
        let pool = msg0(class("NSAutoreleasePool"), sel("new"));
        let app = msg0(class("NSApplication"), sel("sharedApplication"));
        assert!(!app.is_null(), "NSApplication unavailable");
        msg_int(
            app,
            sel("setActivationPolicy:"),
            ffi::NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY,
        );
        let target = create_target();
        assert!(!target.is_null(), "failed to create the action target");
        STATE.lock().unwrap().config = Some(load_config_or_default());
        let menu = build_menu(target);
        let bar = msg0(class("NSStatusBar"), sel("systemStatusBar"));
        let item = msg_cgfloat(
            bar,
            sel("statusItemWithLength:"),
            ffi::NS_VARIABLE_STATUS_ITEM_LENGTH,
        );
        msg_void_id(item, sel("setMenu:"), menu);
        {
            let mut state = STATE.lock().unwrap();
            state.app = Handle(app);
            state.status_item = Handle(item);
            state.target = Handle(target);
        }
        append_log("panel click proof: app bootstrapped");

        // Open the panel through the same path the menu action uses
        // (show_settings), then drive Save through the button's real action.
        show_settings();
        let panel = STATE.lock().unwrap().settings.expect("panel built");
        append_log(&format!(
            "panel click proof: panel visible={}",
            msg_u8(panel.window.0, sel("isVisible")) != 0
        ));

        // Round 0: the owner's first-run scenario — a fresh panel with ZERO
        // rule rows and only the number fields edited (0.35 / 40) must save
        // through the real click dispatch. The run-created file is then
        // removed so Round 1's absent-file guard stays meaningful.
        set_field_text(panel.outer, "40");
        set_field_text(panel.ratio, "0.35");
        msg_void_id(panel.save.0, sel("performClick:"), std::ptr::null_mut());
        let status0 = STATE.lock().unwrap().status.clone();
        let label0 = field_text(panel.status);
        append_log(&format!(
            "panel click proof: zero-row save status={status0:?} label={label0:?}"
        ));
        let (ok0, detail0) = match Config::load() {
            Ok(config) => {
                let values_ok = config.layout.outer_gap == 40.0
                    && config.layout.inner_gap == 8.0
                    && config.layout.master_ratio == 0.35;
                (
                    status0 == "settings saved" && label0 == status0 && values_ok,
                    format!("status={status0:?} label={label0:?} values_ok={values_ok}"),
                )
            }
            Err(error) => (false, format!("load failed: {error}")),
        };
        append_log(&format!("panel click proof: zero-row round {detail0}"));
        let removed0 = match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(error) => {
                append_log(&format!(
                    "panel click proof: zero-row cleanup failed: {error}"
                ));
                false
            }
        };

        // Round 1: an empty row (no matcher) + a Save click must be refused
        // and leave the (absent) config file absent.
        add_rule_row();
        let rows1 = RULE_ROWS.lock().unwrap().len();
        msg_void_id(panel.save.0, sel("performClick:"), std::ptr::null_mut());
        let status1 = STATE.lock().unwrap().status.clone();
        let absent1 = !path.exists();
        append_log(&format!(
            "panel click proof: invalid round rows={rows1} status={status1:?} file_absent={absent1}"
        ));
        // The message surfaces from the final from_json validation; match its
        // stable prefix, not a per-rule detail wording.
        let ok1 = rows1 == 1 && status1.starts_with("settings invalid:") && absent1;

        // Round 1b: the comma decimal separator the owner typed on this host
        // ("0,35") must be refused with the field named, the panel's status
        // line must mirror the chip, and the file must stay absent. The ratio
        // text is restored afterwards so Round 2 saves a valid body.
        set_field_text(panel.ratio, "0,35");
        msg_void_id(panel.save.0, sel("performClick:"), std::ptr::null_mut());
        let status1b = STATE.lock().unwrap().status.clone();
        let label1b = field_text(panel.status);
        let absent1b = !path.exists();
        append_log(&format!(
            "panel click proof: comma round status={status1b:?} label={label1b:?} file_absent={absent1b}"
        ));
        set_field_text(panel.ratio, "0.35");
        // Match the field-named prefix, not the exact suggestion wording.
        let ok1b = status1b.starts_with("invalid number in master_ratio:")
            && label1b == status1b
            && absent1b;

        // Round 2: fill the row at the row surface (bundle id + the exclude
        // policy) + a Save click must save the canonical body (one bundle
        // rule) with owner-only 0600.
        {
            let row = RULE_ROWS.lock().unwrap()[0];
            set_field_text(row.entry, "com.example.proof");
            select_popup_title(row.policy, "exclude");
        }
        msg_void_id(panel.save.0, sel("performClick:"), std::ptr::null_mut());
        let status2 = STATE.lock().unwrap().status.clone();
        let label2 = field_text(panel.status);
        append_log(&format!(
            "panel click proof: valid round rows={} status={status2:?} label={label2:?}",
            RULE_ROWS.lock().unwrap().len()
        ));
        let (ok2, detail) = match Config::load() {
            Ok(config) => {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path)
                    .map(|meta| meta.permissions().mode() & 0o777)
                    .unwrap_or(0);
                let rules_ok = config.rules.len() == 1
                    && config.rules[0].bundle.as_deref() == Some("com.example.proof")
                    && config.rules[0].dialogs == DialogPolicy::Exclude;
                let mode_ok = crate::config::is_owner_only_mode(mode);
                let label_ok = label2 == status2;
                (
                    status2 == "settings saved" && label_ok && rules_ok && mode_ok,
                    format!(
                        "rules_ok={rules_ok} mode={mode:o} mode_ok={mode_ok} label_ok={label_ok}"
                    ),
                )
            }
            Err(error) => (false, format!("load failed: {error}")),
        };
        append_log(&format!("panel click proof: saved verification {detail}"));

        // Hide the panel so the run leaves no visible window behind, then
        // clean up: the file this run created returns to absence.
        msg_void(panel.window.0, sel("orderOut:"));
        let cleaned = match std::fs::remove_file(&path) {
            Ok(()) => {
                append_log("panel click proof: cleanup: removed the file this run created");
                true
            }
            Err(error) => {
                append_log(&format!("panel click proof: cleanup failed: {error}"));
                false
            }
        };
        let exit_code = if ok0 && removed0 && ok1 && ok1b && ok2 && cleaned {
            K_SELF_TEST_EXIT_OK
        } else {
            K_SELF_TEST_EXIT_RESTORE_FAILED
        };
        append_log(&format!("panel click proof: exit {exit_code}"));
        msg_void(pool, sel("drain"));
        exit_code
    }
}

#[cfg(test)]
mod pre_run_classification_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hypr-rust-pre-run-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create isolated test directory");
        path
    }

    #[test]
    fn missing_path_is_missing_without_loading_config() {
        let dir = scratch_dir();
        let path = dir.join("config.json");
        let result = classify_pre_run_at(&path, || -> Result<Config, ConfigError> {
            panic!("a missing path must not load config")
        });
        assert!(matches!(result, PreRun::Missing));
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn live_symlink_is_unsafe_and_left_untouched() {
        use std::os::unix::fs::symlink;

        let dir = scratch_dir();
        let target = dir.join("target.json");
        let path = dir.join("config.json");
        std::fs::write(&target, b"original target").unwrap();
        symlink(&target, &path).unwrap();

        let result = classify_pre_run_at(&path, || -> Result<Config, ConfigError> {
            panic!("a symlink must be rejected before loading config")
        });
        assert!(matches!(result, PreRun::Unsafe(_)));
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        assert_eq!(std::fs::read(&target).unwrap(), b"original target");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dangling_symlink_is_unsafe_and_left_untouched() {
        use std::os::unix::fs::symlink;

        let dir = scratch_dir();
        let target = dir.join("missing-target.json");
        let path = dir.join("config.json");
        symlink(&target, &path).unwrap();

        let result = classify_pre_run_at(&path, || -> Result<Config, ConfigError> {
            panic!("a dangling symlink must not be treated as missing config")
        });
        assert!(matches!(result, PreRun::Unsafe(_)));
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod settings_number_validation_tests {
    use super::is_json_number;

    #[test]
    fn accepts_every_number_the_json_parser_reads() {
        for text in ["0", "0.35", "40", "12", "-0.5", "1e5", "1.5E-2", "007"] {
            assert!(is_json_number(text), "{text:?} should be accepted");
        }
    }

    #[test]
    fn refuses_everything_the_json_parser_cannot_read() {
        // ".35", "0.35e" and "0.35." are the character-clean values that
        // previously broke the generated body ("config is not valid JSON");
        // "0,35" is the comma decimal separator typed on the owner's host.
        for text in [
            "",
            " ",
            ".35",
            "0.35e",
            "0.35.",
            "0,35",
            "+1",
            "0..5",
            "abc",
            "0.35 0.35",
            "1e",
        ] {
            assert!(!is_json_number(text), "{text:?} should be refused");
        }
    }
}

#[cfg(test)]
mod strict_status_text_tests {
    use super::{push_unavailable, strict_status_text};
    use crate::adapter::{AxError, WindowKey};
    use crate::batch::{BatchResult, StrictBatchResult, StrictStatus, WindowOutcome};
    use layout_core::Rect;

    fn success() -> StrictBatchResult {
        StrictBatchResult {
            result: BatchResult {
                outcomes: vec![
                    (
                        WindowKey(1),
                        WindowOutcome::Applied {
                            requested: Rect::new(10.0, 10.0, 200.0, 100.0),
                            actual: Rect::new(10.0, 10.0, 200.0, 100.0),
                        },
                    ),
                    (
                        WindowKey(2),
                        WindowOutcome::Applied {
                            requested: Rect::new(220.0, 10.0, 200.0, 100.0),
                            actual: Rect::new(220.0, 10.0, 200.0, 100.0),
                        },
                    ),
                ],
                failed_at: None,
                error: None,
                rollback_all_restored: true,
            },
            refused_before_write: false,
        }
    }

    fn refusal(before_write: bool) -> StrictBatchResult {
        StrictBatchResult {
            result: BatchResult {
                outcomes: Vec::new(),
                failed_at: Some(WindowKey(2)),
                error: Some(AxError::Failure(-25204)),
                rollback_all_restored: true,
            },
            refused_before_write: before_write,
        }
    }

    fn loud_partial() -> StrictBatchResult {
        StrictBatchResult {
            result: BatchResult {
                outcomes: vec![
                    (
                        WindowKey(1),
                        WindowOutcome::RolledBack {
                            restored: Rect::new(0.0, 0.0, 100.0, 50.0),
                            actual: None,
                        },
                    ),
                    (
                        WindowKey(3),
                        WindowOutcome::RolledBack {
                            restored: Rect::new(300.0, 0.0, 100.0, 50.0),
                            actual: Some(Rect::new(300.0, 40.0, 100.0, 50.0)),
                        },
                    ),
                ],
                failed_at: Some(WindowKey(2)),
                error: Some(AxError::Failure(-25200)),
                rollback_all_restored: false,
            },
            refused_before_write: false,
        }
    }

    #[test]
    fn success_names_the_applied_count_and_never_claims_atomicity() {
        let text = strict_status_text("Reflow", &success(), 3, 1);
        assert!(text.starts_with("Reflow: tiled 2 window(s)"));
        assert!(text.contains("3 excluded"));
        assert!(text.contains("1 skipped"));
        assert!(!text.to_lowercase().contains("atomic"));
        assert_eq!(success().status(), StrictStatus::Success);
    }

    #[test]
    fn preflight_refusal_says_zero_windows_touched() {
        let text = strict_status_text("Reflow", &refusal(true), 0, 0);
        assert!(text.contains("refused before write"));
        assert!(text.contains("0 windows touched"));
        assert!(!text.to_lowercase().contains("atomic"));
        assert_eq!(refusal(true).status(), StrictStatus::CleanRefusal);
    }

    #[test]
    fn mid_batch_refusal_says_all_windows_restored() {
        let text = strict_status_text("Reflow", &refusal(false), 2, 0);
        assert!(text.contains("refused"));
        assert!(text.contains("all windows restored"));
        // A non-global key (no pid) names the raw key, never invents an app.
        assert!(text.contains("failed at key 2"));
        assert!(!text.to_lowercase().contains("atomic"));
        assert_eq!(refusal(false).status(), StrictStatus::CleanRefusal);
    }

    #[test]
    fn clean_refusal_names_the_failing_app_by_pid() {
        // The real host run: a global key (pid << 32 | index) must surface
        // the app pid so a clamping app can be excluded by rules.
        let mut failed = refusal(false);
        failed.result.failed_at = Some(crate::adapter::global_window_key(477, 1));
        let text = strict_status_text("Reflow", &failed, 0, 0);
        assert!(text.contains("failed at app pid 477 (window 1)"));
        // The preflight refusal names it too.
        let mut before = refusal(true);
        before.result.failed_at = Some(crate::adapter::global_window_key(63181, 0));
        let text = strict_status_text("Reflow", &before, 0, 0);
        assert!(text.contains("failed at app pid 63181 (window 0)"));
        assert!(text.contains("0 windows touched"));
    }

    #[test]
    fn loud_partial_names_observed_frames_and_never_reads_as_success() {
        let text = strict_status_text("Reflow", &loud_partial(), 0, 0);
        assert!(text.contains("PARTIAL STATE"));
        assert!(text.contains("restore unverified"));
        // Every rolled-back key with its observed frame is named honestly.
        assert!(text.contains("key 1"));
        assert!(text.contains("unreadable"));
        assert!(text.contains("key 3"));
        assert!(text.contains("300.0"));
        assert!(!text.contains("tiled"));
        assert!(!text.to_lowercase().contains("atomic"));
        assert_eq!(loud_partial().status(), StrictStatus::LoudPartial);
    }

    #[test]
    fn unavailable_apps_are_named_by_pid_and_absent_stays_clean() {
        // §2.1 amendment: skipped apps appear by pid in any status line…
        let mut text = strict_status_text("Reflow", &success(), 1, 0);
        push_unavailable(&mut text, &[477, 63181]);
        assert!(text.contains(", unavailable skipped: pid 477, 63181"));
        assert!(text.starts_with("Reflow: tiled"));
        // …and with no skipped app the line is untouched.
        let mut clean = strict_status_text("Reflow", &success(), 0, 0);
        push_unavailable(&mut clean, &[]);
        assert!(!clean.contains("unavailable"));
    }
}
