//! Minimal macOS menu-bar shell (Gap 3).
//!
//! A Rust `NSStatusItem` with a menu: a status line, explicit reflow commands,
//! pause/resume, an Accessibility status line, diagnostics, and quit. All
//! AppKit/Objective-C `unsafe` FFI lives in [`crate::ffi`]; this module only
//! composes documented wrappers behind a small surface.
//!
//! Auto-tiling stays **off**: nothing is applied unless the user picks a reflow
//! command. Quit terminates the app and leaves windows at their last
//! OS-accepted position. The shell never reads window titles and never acts on
//! a window outside the registered fixture.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Mutex;

use layout_core::{DisplayId, PlatformSnapshot, Rect, WindowRole};

use crate::adapter::MacAxAdapter;
use crate::batch;
use crate::config::{
    dialog_policy_for, parse_rules_text, rules_json, Config, ConfigError, DialogPolicy, WindowRule,
};
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
objc_msg!(msg_view_init(rect: ffi::CGRect));
objc_msg!(msg_popup_init(rect: ffi::CGRect, pulls_down: bool));
objc_msg!(msg_window_init(
    rect: ffi::CGRect,
    style_mask: isize,
    backing: i32,
    defer_flag: u8
));

/// Send an Objective-C message with a non-object return type.
///
/// SAFETY: see [`msg0`].
macro_rules! objc_msg_ret {
    ($name:ident($($arg:ident : $ty:ty),*) -> $ret:ty) => {
        unsafe fn $name(obj: Id, s: Sel $(, $arg: $ty)*) -> $ret {
            let f: unsafe extern "C" fn(Id, Sel $(, $arg)*) -> $ret = unsafe {
                std::mem::transmute(ffi::objc_msgSend as unsafe extern "C" fn(Id, Sel) -> Id)
            };
            unsafe { f(obj, s $(, $arg)*) }
        }
    };
}

objc_msg_ret!(msg_isize() -> isize);
objc_msg_ret!(msg_u8() -> u8);
objc_msg_ret!(msg_cstr() -> *const c_char);

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
    rules: Handle,
}

// Field layout inside the panel content view (content coordinates, origin at
// the bottom-left; the title bar sits above). The rules editor sits between
// the ratio row and the checkboxes; Save saves what is on screen, rules from
// the editor text.
const K_SETTINGS_RECTS: [(f64, f64, f64, f64); 15] = [
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
    (16.0, 66.0, 308.0, 96.0),   // rules text view
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

/// The loaded config, or the defaults before one is loaded.
fn current_config() -> Config {
    STATE.lock().unwrap().config.clone().unwrap_or_default()
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
    perform_reflow();
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
    let (popup, outer, inner, ratio, auto, dialogs, rules_text) = unsafe {
        let rules_text = rules_view_text(panel.rules);
        (
            popup_title(panel.popup),
            field_text(panel.outer),
            field_text(panel.inner),
            field_text(panel.ratio),
            checkbox_on(panel.auto_tiling),
            checkbox_on(panel.dialogs),
            rules_text,
        )
    };
    // The rules editor is authoritative for rules: the typed text must parse
    // as a valid rules list (the same entry validation a file load runs), and
    // only the canonical regenerated JSON is embedded — raw editor text never
    // reaches the body, so a typed quote cannot break the JSON structure.
    let rules = match parse_rules_text(&rules_text) {
        Ok(rules) => rules,
        Err(error) => {
            set_status(&format!("settings invalid: rules ({error})"));
            refresh_menu();
            return;
        }
    };
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
                STATE.lock().unwrap().config = Some(config);
                set_status("settings saved");
            }
            Err(error) => set_status(&format!("settings save failed: {error}")),
        },
        Err(error) => {
            set_status(&format!("settings invalid: {error}"));
        }
    }
    refresh_menu();
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
    // SAFETY: main-thread calls on live handles; populate before showing.
    unsafe {
        populate_settings(panel, &config);
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
    let app = STATE.lock().unwrap().app;
    if !app.is_null() {
        // SAFETY: `app` is the live NSApplication instance.
        unsafe { msg_void_id(app.0, sel("terminate:"), std::ptr::null()) };
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
/// SAFETY: the panel handles must be live AppKit objects.
unsafe fn populate_settings(panel: SettingsPanel, config: &Config) {
    unsafe {
        select_popup_title(panel.popup, config.kind_name());
        set_field_text(panel.outer, &fmt_num(config.layout.outer_gap));
        set_field_text(panel.inner, &fmt_num(config.layout.inner_gap));
        set_field_text(panel.ratio, &fmt_num(config.layout.master_ratio));
        set_checkbox(panel.auto_tiling, config.auto_tiling);
        set_checkbox(panel.dialogs, config.include_dialogs);
        // The editor shows the canonical JSON of the effective rules; Save
        // re-validates whatever the user typed there.
        set_rules_text(panel.rules, &rules_json(&config.rules));
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

        // Rules editor: a scrollable plain-text view holding the JSON array.
        make_label(content, K_SETTINGS_RECTS[13], "Rules (JSON array)");
        let rules = make_rules_text(content, K_SETTINGS_RECTS[14]);

        // Save / Close: the window retains them; the actions live on `target`.
        let _save = make_button(
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

        SettingsPanel {
            window: Handle(window),
            popup: Handle(popup),
            outer,
            inner,
            ratio,
            auto_tiling,
            dialogs,
            rules,
        }
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
unsafe fn make_label(content: Id, place: (f64, f64, f64, f64), title: &str) {
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
    }
}

/// Allocate a scrollable plain-text view for the rules JSON editor and add it
/// to `content`. Returns the text view handle (the scroll view is discarded —
/// the window retains it).
///
/// SAFETY: main thread; `content` must be a live view.
unsafe fn make_rules_text(content: Id, place: (f64, f64, f64, f64)) -> Handle {
    // SAFETY: documented initialisers; the classic vertically-resizable text
    // view inside a scroll view recipe, all documented selectors.
    let text = unsafe {
        let scroll = {
            let alloc = msg0(class("NSScrollView"), sel("alloc"));
            let scroll = msg_view_init(alloc, sel("initWithFrame:"), rect(place));
            msg_bool(scroll, sel("setHasVerticalScroller:"), true);
            msg_bool(scroll, sel("setAutohidesScrollers:"), true);
            scroll
        };
        let text = {
            let alloc = msg0(class("NSTextView"), sel("alloc"));
            let text = msg_view_init(alloc, sel("initWithFrame:"), rect(place));
            msg_bool(text, sel("setRichText:"), false);
            msg_bool(text, sel("setEditable:"), true);
            msg_bool(text, sel("setVerticallyResizable:"), true);
            msg_bool(text, sel("setHorizontallyResizable:"), false);
            msg_int(text, sel("setAutoresizingMask:"), 16);
            // The container width follows the text view, so the editor wraps
            // at the panel width; growth is vertical only.
            let container = msg0(text, sel("textContainer"));
            msg_bool(container, sel("setWidthTracksTextView:"), true);
            text
        };
        msg_void_id(scroll, sel("setDocumentView:"), text);
        msg_void_id(content, sel("addSubview:"), scroll);
        text
    };
    Handle(text)
}

/// Replace the rules editor's text.
///
/// SAFETY: `view` must be a live NSTextView.
unsafe fn set_rules_text(view: Handle, text: &str) {
    unsafe { msg_void_id(view.0, sel("setString:"), nsstring(text)) };
}

/// Text of the rules editor.
///
/// SAFETY: `view` must be a live NSTextView.
unsafe fn rules_view_text(view: Handle) -> String {
    let value = unsafe { msg0(view.0, sel("string")) };
    unsafe { utf8_string(value) }
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
fn append_log(line: &str) {
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

/// Per-window rule overrides for one reflow: the app's first matching rule
/// decides the dialog policy for that app's Dialog windows only; mandatory
/// roles never appear here (planner gates keep them excluded).
///
/// SAFETY: main thread; AppKit query only (bundle id, never titles).
fn dialog_policy_rules(
    snapshot: &PlatformSnapshot,
    pid: u32,
    rules: &[WindowRule],
    include_dialogs: bool,
) -> Vec<layout_core::WindowDialogRule> {
    let bundle = unsafe { app_bundle_id(pid) };
    let include = match dialog_policy_for(rules, pid, bundle.as_deref()) {
        DialogPolicy::Exclude => false,
        DialogPolicy::Include => true,
        DialogPolicy::Default => include_dialogs,
    };
    snapshot
        .windows
        .iter()
        .filter(|window| window.role == WindowRole::Dialog)
        .map(|window| layout_core::WindowDialogRule {
            window: window.id,
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
    let dialog_rules = dialog_policy_rules(&observation.snapshot, pid, &rules, include_dialogs);
    let plan = match layout_core::plan_snapshot_with_rules(
        &observation.snapshot,
        DisplayId(0),
        kind,
        layout,
        include_dialogs,
        &dialog_rules,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            set_status(&format!("{label}: plan {error:?}"));
            return;
        }
    };
    let targets = match batch::targets_for_plan(&plan, &observation.map) {
        Ok(targets) => targets,
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

/// Register the action-target class and return one instance.
///
/// SAFETY: each `class_addMethod` pairs a registered selector with an
/// `extern "C" fn(Id, Sel)` implementation, encoded `v@:`.
unsafe fn create_target() -> Id {
    let superclass = class("NSObject");
    let name = b"HyprRustShell\0";
    // SAFETY: NSObject is a live class and `name` is a NUL-terminated C string.
    let cls = unsafe { ffi::objc_allocateClassPair(superclass, name.as_ptr() as *const c_char, 0) };
    assert!(!cls.is_null(), "failed to allocate the HyprRustShell class");
    let types = b"v@:\0";
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

fn classify_pre_run() -> PreRun {
    let path = match crate::config::config_path() {
        Ok(path) => path,
        Err(error) => return PreRun::Unsafe(format!("home path unavailable: {error}")),
    };
    let mode = match std::fs::metadata(&path) {
        Ok(meta) => {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o777
        }
        // Missing is a first-class state: the run may write and must restore
        // to no-file at the end.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return PreRun::Missing,
        Err(error) => return PreRun::Unsafe(format!("unreadable: {error}")),
    };
    match crate::config::Config::load() {
        Ok(config) => PreRun::Valid(config),
        Err(crate::config::ConfigError::NotOwnerOnly) => PreRun::Unsafe(format!(
            "not owner-only (mode {mode:o}) or a symlink/foreign uid"
        )),
        Err(error) => PreRun::Unsafe(format!("config invalid: {error}")),
    }
}

pub fn self_test() -> i32 {
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

        // Settings panel + rules editor: open through the same path the menu
        // action uses, read the controls back, save through the Save button's
        // own helper, prove invalid input is refused, then restore defaults
        // for the next run. A seeded rule is in place before the panel opens,
        // so the editor displays it and a save round-trips it.
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
            let rules_text = rules_view_text(panel.rules);
            let visible = msg_u8(panel.window.0, sel("isVisible"));
            append_log(&format!(
                "self-test: panel visible={visible} popup={popup:?} outer={outer} inner={inner} ratio={ratio} auto={auto} dialogs={dialogs} rules={rules_text}"
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
            // 3) Rules edited in the editor: a pid rule replaces the seeded
            // one (text in, entry validation, canonical text out).
            set_field_text(panel.ratio, "0.7");
            if let Some(pid) = fixture::registered_fixture_pid().filter(|pid| process_alive(*pid)) {
                set_rules_text(
                    panel.rules,
                    &format!(r#"[{{"pid": {pid}, "dialogs": "include"}}]"#),
                );
            } else {
                set_rules_text(
                    panel.rules,
                    r#"[{"bundle": "com.example.edited", "dialogs": "include"}]"#,
                );
            }
            save_settings_from_panel();
            append_log(&format!(
                "self-test: rules edit save status = {}",
                STATE.lock().unwrap().status.clone()
            ));
            append_log(&format!(
                "self-test: loaded after rules edit = {:?}",
                Config::load()
            ));
            // 4) Broken rules text is refused; the file keeps the last saved
            // rules list.
            set_rules_text(panel.rules, r#"[{"pid": 1"#);
            save_settings_from_panel();
            append_log(&format!(
                "self-test: broken rules save status = {}",
                STATE.lock().unwrap().status.clone()
            ));
            append_log(&format!(
                "self-test: loaded after broken rules = {:?}",
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
