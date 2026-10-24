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

use layout_core::{DisplayId, Rect};

use crate::adapter::MacAxAdapter;
use crate::batch;
use crate::config::{Config, ConfigError};
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

/// Shell state shared with the Objective-C action callbacks. Menu actions run
/// on the main thread, so a plain `Mutex` is enough.
struct ShellState {
    paused: bool,
    app: Handle,
    status_item: Handle,
    target: Handle,
    config: Option<Config>,
    status: String,
}

static STATE: Mutex<ShellState> = Mutex::new(ShellState {
    paused: false,
    app: Handle::null(),
    status_item: Handle::null(),
    target: Handle::null(),
    config: None,
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
    match Config::default().save() {
        Ok(()) => set_status("settings reset to defaults"),
        Err(error) => set_status(&format!("reset failed: {error}")),
    }
}

extern "C" fn action_diagnostics(_this: Id, _cmd: Sel) {
    let summary = status_line();
    append_log(&summary);
}

extern "C" fn action_quit(_this: Id, _cmd: Sel) {
    let app = STATE.lock().unwrap().app;
    if !app.is_null() {
        // SAFETY: `app` is the live NSApplication instance.
        unsafe { msg_void_id(app.0, sel("terminate:"), std::ptr::null()) };
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
    let config = current_config();
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
        add_action(menu, target, "Pause", "togglePause:");
        msg_void_id(menu, sel("addItem:"), separator());
        add_status(menu, &settings_line());
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
    let auto_tiling = if config.auto_tiling { "on" } else { "off" };
    let dialogs = if config.include_dialogs { "in" } else { "out" };
    format!(
        "settings: layout={} gaps={}/{} ratio={} auto-tiling={auto_tiling} dialogs={dialogs}",
        config.kind_name(),
        fmt_num(config.layout.outer_gap),
        fmt_num(config.layout.inner_gap),
        fmt_num(config.layout.master_ratio),
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
    let (kind, layout, include_dialogs, label) = {
        let config = current_config();
        let (kind, layout) = config.plan_config();
        (
            kind,
            layout,
            config.include_dialogs,
            config.kind_name().to_string(),
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
    let plan = match layout_core::plan_snapshot(
        &observation.snapshot,
        DisplayId(0),
        kind,
        layout,
        include_dialogs,
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
    let status = if let Some(error) = &result.error {
        format!(
            "{label}: failed at key {:?} ({error})",
            result.failed_at.map(|key| key.0)
        )
    } else {
        format!("{label}: applied {} window(s)", result.outcomes.len())
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
pub fn self_test() {
    unsafe {
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
        msg_void(pool, sel("drain"));
    }
}
