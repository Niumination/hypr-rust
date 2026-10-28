//! Raw FFI declarations for macOS Accessibility (HIServices) and
//! CoreFoundation. This module is the ONLY place in the crate that touches
//! `unsafe` system calls; every wrapper documents its safety invariants.
//!
//! Handles created via these functions are owned by the caller and must be
//! released with CFRelease (see [`crate::AxWindow`]).

#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_void};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type AXUIElementRef = *const c_void;
pub type AXValueRef = *const c_void;
/// Opaque Accessibility observer ref (a CFType; released with CFRelease).
pub type AXObserverRef = *const c_void;
/// Opaque CoreFoundation run-loop refs (borrowed where noted per function).
pub type CFRunLoopRef = *const c_void;
pub type CFRunLoopSourceRef = *const c_void;
pub type CFRunLoopTimerRef = *const c_void;

/// Darwin `Boolean` (0 = false, non-zero = true).
pub type Boolean = u8;

/// CFString encoding for UTF-8 (kCFStringEncodingUTF8).
pub const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// AXValue type tags (AXValue.h).
pub const K_AX_VALUE_CG_POINT_TYPE: c_int = 1;
pub const K_AX_VALUE_CG_SIZE_TYPE: c_int = 2;

// open(2) flags (fcntl.h).
pub const O_RDWR: c_int = 0x0002;
pub const O_CREAT: c_int = 0x0200;
// flock(2) operations (sys/file.h).
pub const LOCK_EX: c_int = 0x02;
pub const LOCK_NB: c_int = 0x04;
// errno: resource temporarily unavailable (EWOULDBLOCK == EAGAIN on macOS).
pub const K_ERRNO_EWOULDBLOCK: i32 = 35;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// True when the current process holds Accessibility permission.
    /// Safety: no arguments, no state; callable any time.
    pub fn AXIsProcessTrusted() -> Boolean;

    /// Create an AX application element for a pid.
    /// Safety: `pid` must be a valid pid_t; returns an owned ref or null.
    pub fn AXUIElementCreateApplication(pid: c_int) -> AXUIElementRef;

    /// Copy the pid of the process that owns an AX element. Returns an AXError
    /// code (0 = success). Safety: `elem` must be a live AXUIElementRef;
    /// `pid` receives the owning pid.
    pub fn AXUIElementGetPid(elem: AXUIElementRef, pid: *mut c_int) -> c_int;

    /// Copy an attribute value. On success `*value` holds an owned CFTypeRef
    /// that the caller MUST CFRelease. Returns an AXError code (0 = success).
    /// Safety: `elem` must be a live AXUIElementRef; `attr` a valid CFString.
    pub fn AXUIElementCopyAttributeValue(
        elem: AXUIElementRef,
        attr: CFStringRef,
        value: *mut CFTypeRef,
    ) -> c_int;

    /// Set an attribute value. `value` is borrowed (not released by callee).
    /// Safety: `elem` live; `value` must match the attribute's expected type.
    pub fn AXUIElementSetAttributeValue(
        elem: AXUIElementRef,
        attr: CFStringRef,
        value: CFTypeRef,
    ) -> c_int;

    /// Perform an AX action (e.g. raise). Safety: `elem` live, `action` valid.
    pub fn AXUIElementPerformAction(elem: AXUIElementRef, action: CFStringRef) -> c_int;

    /// Extract a CGPoint/CGSize from an AXValue. Safety: `value` must be an
    /// AXValue of the requested type; `out` must point to a correctly sized
    /// destination (2×CGFloat).
    pub fn AXValueGetValue(value: AXValueRef, ax_type: c_int, out: *mut c_void) -> Boolean;

    /// Create an AXValue wrapping a CGPoint/CGSize. Safety: `value` must
    /// point to data matching `ax_type`; returns an owned ref or null.
    pub fn AXValueCreate(ax_type: c_int, value: *const c_void) -> AXValueRef;

    /// Create an AX observer for the app with pid `application`
    /// (AXUIElement.h `AXObserverCreate`). On success `out` receives an
    /// owned ref (CF_RETURNS_RETAINED — CFRelease). AXError code
    /// (0 = success). Safety: `callback` must be a valid `extern "C"`
    /// function that outlives the observer; `out` a valid out-pointer.
    pub fn AXObserverCreate(
        application: c_int,
        callback: AXObserverCallback,
        out: *mut AXObserverRef,
    ) -> c_int;

    /// Start observing `notification` on `element`; `refcon` is passed to
    /// the callback verbatim (AXUIElement.h `AXObserverAddNotification`).
    /// AXError code (0 = success). Safety: `observer` owned by the caller;
    /// `element` live; `notification` a valid CFString naming the
    /// notification (e.g. [`K_AX_WINDOW_CREATED_NOTIFICATION`] via
    /// [`cf_string_from_static`]); `refcon` must outlive the registration.
    pub fn AXObserverAddNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> c_int;

    /// Stop observing one notification added with `AXObserverAddNotification`
    /// (AXUIElement.h). AXError code (0 = success). Safety: the same refs,
    /// in the same state, as the matching add call.
    pub fn AXObserverRemoveNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
    ) -> c_int;

    /// The run-loop source that delivers this observer's callbacks; add it
    /// with [`CFRunLoopAddSource`] (AXUIElement.h — the SDK's modern
    /// replacement for the removed `AXObserverAddToRunLoop`, verified
    /// absent from HIServices.tbd). Borrowed — do not release.
    /// Safety: `observer` must be a live owned AXObserverRef.
    pub fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> CFRunLoopSourceRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    /// Release a CF object. Safety: `cf` must be a live owned CFTypeRef and
    /// must not be used afterwards.
    pub fn CFRelease(cf: CFTypeRef);

    /// Retain a CF object (bumps refcount). Safety: `cf` live CFTypeRef.
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;

    /// The main thread's run loop — the one `NSApplication` drives and the
    /// same thread the Carbon hotkey handler and menu actions run on.
    /// Borrowed ref, do not release. Safety: no arguments.
    pub fn CFRunLoopGetMain() -> CFRunLoopRef;

    /// Add a run-loop source in `mode` (CFRunLoop.h). Safety: `run_loop`
    /// live (e.g. `CFRunLoopGetMain()`); `source` borrowed from its owner;
    /// `mode` a valid CFString (e.g. [`K_CF_RUN_LOOP_DEFAULT_MODE`] via
    /// [`cf_string_from_static`]).
    pub fn CFRunLoopAddSource(
        run_loop: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFStringRef,
    );

    /// Remove a run-loop source (pair of `CFRunLoopAddSource`). Safety: as
    /// above; after removal the source must deliver no further callbacks.
    pub fn CFRunLoopRemoveSource(
        run_loop: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFStringRef,
    );

    /// Absolute time in seconds since the reference epoch (2001-01-01)
    /// (CFAbsoluteTimeGetCurrent). Safety: no arguments.
    pub fn CFAbsoluteTimeGetCurrent() -> f64;

    /// Create a run-loop timer (CFRunLoop.h). `fire_date`/`interval` are
    /// CFAbsoluteTime seconds (`interval` 0 = one-shot); returns an owned
    /// ref (CFRelease, or ownership passes to the run loop once added).
    /// Safety: `callout` must be a valid `extern "C"` fn outliving the
    /// timer; `context` a valid context pointer or null.
    pub fn CFRunLoopTimerCreate(
        allocator: CFTypeRef,
        fire_date: f64,
        interval: f64,
        flags: usize,
        order: isize,
        callout: CFRunLoopTimerCallBack,
        context: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;

    /// Add a timer to a run loop in `mode` (CFRunLoop.h). Safety:
    /// `run_loop` live; `timer` an owned timer ref; `mode` a valid CFString.
    pub fn CFRunLoopAddTimer(run_loop: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);

    /// Invalidate a timer: it fires no further and is removed from every
    /// run loop (CFRunLoop.h). Safety: `timer` a live timer ref.
    pub fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);

    /// Structural equality of two CF objects (CFBase.h). Safety: both refs
    /// must be live CF objects of comparable types.
    pub fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> Boolean;

    /// Caller's process id (unistd.h, libSystem). Safety: no arguments.
    pub fn getpid() -> c_int;

    /// Create a CFString from a C string. Safety: `cstr` must be a valid
    /// NUL-terminated string; returns an owned ref or null.
    pub fn CFStringCreateWithCString(
        alloc: CFTypeRef,
        cstr: *const c_char,
        encoding: u32,
    ) -> CFStringRef;

    /// Send a signal to a process. With `sig` = 0, performs an existence
    /// check only (no signal sent). Returns 0 on success, -1 on error.
    /// Safety: standard POSIX semantics; `pid` must be a valid process id.
    pub fn kill(pid: c_int, sig: c_int) -> c_int;

    /// Thread-local errno (libSystem). Safety: returns a valid pointer to
    /// the calling thread's errno value; read immediately after a failing
    /// syscall.
    pub fn __error() -> *mut c_int;

    /// Open a file (open(2)). Safety: `path` must be a valid NUL-terminated
    /// C string; `flags`/`mode` follow POSIX open(2). Returns the fd or -1.
    pub fn open(path: *const c_char, flags: c_int, mode: u32) -> c_int;

    /// Close a file descriptor (close(2)). Safety: `fd` must be a valid open
    /// file descriptor.
    pub fn close(fd: c_int) -> c_int;

    /// Current user id (getuid(2)). Safety: no arguments, no state.
    pub fn getuid() -> u32;

    /// Apply/release an advisory lock on an open file (flock(2)). Safety:
    /// `fd` must be a valid open file descriptor. Returns 0 on success, -1 on
    /// error (with errno set, e.g. EWOULDBLOCK for a non-blocking conflict).
    pub fn flock(fd: c_int, operation: c_int) -> c_int;

    /// Number of elements in a CFArray. Safety: `array` live CFArrayRef.
    pub fn CFArrayGetCount(array: CFTypeRef) -> isize;
    /// Element at index (borrowed). Safety: `array` live, `idx` in bounds.
    pub fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: isize) -> CFTypeRef;

    /// Type id of any live CF object. Safety: `cf` live, non-null CFTypeRef.
    pub fn CFGetTypeID(cf: CFTypeRef) -> usize;

    /// Type id of the CFBoolean singleton type. Safety: no arguments.
    pub fn CFBooleanGetTypeID() -> usize;

    /// Value of a CFBoolean (0 = false, non-zero = true). Safety: `boolean`
    /// must be a live CFBooleanRef.
    pub fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;

    /// Copy a CFString's bytes into a caller buffer. `size` is the buffer
    /// capacity in bytes; the result is UTF-8 plus a terminating NUL.
    /// Returns false when the buffer was too small or the string invalid.
    /// Safety: `buffer` must be writable for `size` bytes and `string` must be
    /// a live CFStringRef.
    pub fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> Boolean;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// Main display identifier. Safety: no arguments, no state.
    pub fn CGMainDisplayID() -> u32;

    /// Bounds of a display in global display coordinates (origin at the
    /// top-left of the main display). Safety: `display` must be a valid
    /// display identifier.
    pub fn CGDisplayBounds(display: u32) -> CGRect;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    /// Exported AppKit constants (NSWorkspace.h): the launch/terminate
    /// notification names and the userInfo key carrying the
    /// `NSRunningApplication`. Static NSString pointers — read once per
    /// event on the main thread.
    pub static NSWorkspaceDidLaunchApplicationNotification: CFStringRef;
    pub static NSWorkspaceDidTerminateApplicationNotification: CFStringRef;
    pub static NSWorkspaceApplicationKey: CFStringRef;
}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

extern "C" {
    /// Class lookup. Safety: `name` must be a valid NUL-terminated class name.
    pub fn objc_getClass(name: *const c_char) -> *const c_void;

    /// Selector lookup. Safety: `name` must be a valid NUL-terminated selector.
    pub fn sel_registerName(name: *const c_char) -> *const c_void;

    /// Objective-C message send with no arguments. The C function is variadic;
    /// the other arities live in the `objc_msg_send` submodules below because
    /// Rust rejects redeclaring one symbol with different signatures inside one
    /// module. Safety: `obj` must be a live object (or null) and `sel` a
    /// registered selector.
    pub fn objc_msgSend(obj: *const c_void, sel: *const c_void) -> *const c_void;
    /// Allocate a class pair for a dynamically registered class. Safety:
    /// `superclass` must be a live Class and `name` a valid C string.
    pub fn objc_allocateClassPair(
        superclass: *const c_void,
        name: *const c_char,
        extra_bytes: usize,
    ) -> *const c_void;

    /// Register a previously allocated class pair. Safety: `class` must be a
    /// live Class returned by `objc_allocateClassPair`.
    pub fn objc_registerClassPair(class: *const c_void);

    /// Add a method implementation to a class. Safety: `class` live, `name` a
    /// registered selector, `imp` a matching implementation, `types` a valid
    /// Objective-C type encoding for the method.
    pub fn class_addMethod(
        class: *const c_void,
        name: *const c_void,
        imp: *const c_void,
        types: *const c_char,
    ) -> Boolean;

    /// Instantiate a class. Safety: `class` must be a registered Class.
    pub fn class_createInstance(class: *const c_void, extra_bytes: usize) -> *const c_void;
}

/// `CGFloat` is `f64` on both supported architectures.
pub type CGFloat = f64;

/// `NSApplicationActivationPolicyAccessory`: no Dock icon, no menu bar.
pub const NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY: isize = 1;

/// `NSVariableStatusItemLength` (the item sizes itself to its content).
pub const NS_VARIABLE_STATUS_ITEM_LENGTH: CGFloat = -1.0;

/// Reserved vertical space at the top of the display for the system menu bar,
/// in points. `CGDisplayBounds` reports the full display; the adapter does not
/// query `NSScreen.visibleFrame`, so this documented constant is the only
/// inset applied.
pub const K_MENU_BAR_INSET: CGFloat = 30.0;

/// CoreGraphics point. `#[repr(C)]` matches the C layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

/// CoreGraphics size. `#[repr(C)]` matches the C layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

/// CoreGraphics rectangle. `#[repr(C)]` matches the C layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

impl CGRect {
    /// Build a `Rect`-like rectangle from origin and size.
    pub fn rect(&self) -> layout_core::Rect {
        layout_core::Rect::new(
            self.origin.x,
            self.origin.y,
            self.size.width,
            self.size.height,
        )
    }
}

/// Build a CFString from a static C string literal (the Rust equivalent of
/// the SDK's `CFSTR("...")` macros such as kAXWindowsAttribute).
///
/// The returned string is an owned ref. Intended for process-lifetime
/// constants (see [`crate::adapter`]); callers that use it transiently must
/// CFRelease it.
pub fn cf_string_from_static(text: &[u8]) -> CFStringRef {
    // SAFETY: `text` is a NUL-terminated static byte string; the encoding
    // constant is the documented kCFStringEncodingUTF8 value.
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            text.as_ptr() as *const c_char,
            K_CF_STRING_ENCODING_UTF8,
        )
    }
}

// MARK: - Accessibility observer (auto-tiling, ADR-0008)

/// AXObserver callback (`AXUIElement.h` `AXObserverCallback`): delivered on
/// the run loop the observer's source was added to, once per notification,
/// with the affected element, the notification name and the registration's
/// `refcon`.
pub type AXObserverCallback = extern "C" fn(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
);

/// Run-loop timer callback (`CFRunLoop.h` `CFRunLoopTimerCallBack`).
pub type CFRunLoopTimerCallBack = extern "C" fn(timer: CFRunLoopTimerRef, info: *mut c_void);

/// Run-loop timer context (`CFRunLoop.h`, 5 fields in this SDK — no
/// `equal`, unlike the observer context). Pass a null context pointer when
/// no user info is needed.
#[repr(C)]
pub struct CFRunLoopTimerContext {
    pub version: isize,
    pub info: *mut c_void,
    pub retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub release: Option<unsafe extern "C" fn(*const c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const c_void) -> CFStringRef>,
}

/// Notification name `AXWindowCreated` (`AXNotificationConstants.h` — the
/// SDK defines these as `CFSTR` macros, not exported symbols; build the
/// CFString with [`cf_string_from_static`]). Observed on the **app**
/// element (ADR-0008: a window opened).
pub const K_AX_WINDOW_CREATED_NOTIFICATION: &[u8] = b"AXWindowCreated\0";

/// Notification name `AXUIElementDestroyed` (`AXNotificationConstants.h`).
/// Observed on **window** elements (ADR-0008: a window closed).
pub const K_AX_UI_ELEMENT_DESTROYED_NOTIFICATION: &[u8] = b"AXUIElementDestroyed\0";

/// Run-loop mode `kCFRunLoopDefaultMode` (`CFRunLoop.h` — the constant's
/// value is literally this string, so [`cf_string_from_static`] produces an
/// equal mode CFString). This is the mode `NSApplication`'s main run loop
/// runs in. Build with [`cf_string_from_static`].
pub const K_CF_RUN_LOOP_DEFAULT_MODE: &[u8] = b"kCFRunLoopDefaultMode\0";

// MARK: - Carbon Event Manager (global hotkey, ADR-0007)

/// Opaque Carbon event-manager refs (`EventTargetRef`, `EventRef`,
/// `EventHandlerCallRef`, `EventHandlerRef`, `EventHotKeyRef`). Ownership:
/// `GetApplicationEventTarget` returns the process-lifetime target (not
/// released by us); `InstallEventHandler`/`RegisterEventHotKey` return refs
/// released with `RemoveEventHandler`/`UnregisterEventHotKey`.
pub type EventRef = *mut c_void;
pub type EventHandlerCallRef = *mut c_void;
pub type EventTargetRef = *mut c_void;
pub type EventHandlerRef = *mut c_void;
pub type EventHotKeyRef = *mut c_void;

/// `OSStatus` (MacTypes.h) — `noErr = 0`, negatives are errors.
pub type OSStatus = i32;
/// `OptionBits` (MacTypes.h).
pub type OptionBits = u32;

/// `kEventClassKeyboard = 'keyb'` (CarbonEvents.h, big-endian four-char).
pub const K_EVENT_CLASS_KEYBOARD: u32 = 0x6B65_7962;
/// `kEventHotKeyPressed = 5` (CarbonEvents.h).
pub const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// `kEventParamDirectObject = '----'` — where the pressed event carries the
/// `EventHotKeyID` (CarbonEvents.h).
pub const K_EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
/// `typeEventHotKeyID = 'hkid'` (CarbonEvents.h).
pub const K_TYPE_EVENT_HOT_KEY_ID: u32 = 0x686B_6964;
/// `eventHotKeyExistsErr = -9878` (MacErrors.h): the chord is already
/// registered by this or another process — the conflict case (ADR-0007 §3).
pub const OSSTATUS_EVENT_HOT_KEY_EXISTS: OSStatus = -9878;
/// `eventNotHandledErr = -9874` (MacErrors.h): handler declines the event.
pub const OSSTATUS_EVENT_NOT_HANDLED: OSStatus = -9874;
/// `noErr` (MacTypes.h).
pub const OSSTATUS_NO_ERR: OSStatus = 0;

// Carbon virtual key/modifier codes (Events.h / CarbonEvents.h): the
// modifiers are a bitfield, not a small enum — control 4096 | option 2048 |
// cmd 256; `kVK_ANSI_R = 0x0F`.
pub const K_MOD_CONTROL: u32 = 4096;
pub const K_MOD_OPTION: u32 = 2048;
pub const K_MOD_COMMAND: u32 = 256;
/// `kVK_ANSI_R` (Events.h).
pub const K_VK_ANSI_R: u32 = 0x0F;

/// `EventHotKeyID` (CarbonEvents.h): signature identifies our application,
/// id identifies which of our hotkeys fired. Passed **by value** to
/// `RegisterEventHotKey` and read back from the event by value.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventHotKeyID {
    pub signature: u32,
    pub id: u32,
}

/// `EventTypeSpec` (CarbonEvents.h): event class + kind filter.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EventTypeSpec {
    pub class: u32,
    pub kind: u32,
}

/// `EventHandlerUPP` in 64-bit Carbon is a plain function pointer
/// (CarbonEvents.h): handler, event, user data passed to `InstallEventHandler`.
pub type EventHandlerUPP = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    /// The process's application event target (not owned; process lifetime).
    pub fn GetApplicationEventTarget() -> EventTargetRef;

    /// Install a handler on the application target. `inNumTypes`/`inList`
    /// filter the classes; `outRef` receives an owned ref for
    /// `RemoveEventHandler`. Safety: `handler` must stay valid until removed.
    /// `ItemCount` is 64-bit (`unsigned long`) on modern macOS.
    pub fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerUPP,
        num_types: u64,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;

    /// Remove a handler previously returned via `out_ref`. Safety: `handler`
    /// must be a live ref; using it afterwards is undefined.
    pub fn RemoveEventHandler(handler: EventHandlerRef) -> OSStatus;

    /// Claim a system-wide hotkey chord. The chord is intercepted **before**
    /// the frontmost app sees it. Returns `eventHotKeyExistsErr` when the
    /// chord is taken. Safety: `out_ref` receives an owned ref for
    /// `UnregisterEventHotKey`.
    pub fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: OptionBits,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;

    /// Release a hotkey ref from `RegisterEventHotKey`. Safety: `hotkey`
    /// must be a live ref; using it afterwards is undefined.
    pub fn UnregisterEventHotKey(hotkey: EventHotKeyRef) -> OSStatus;

    /// Copy a parameter out of an event. Argument order per Carbon Event
    /// Manager Reference: `outActualType` and `outActualSize` may be null;
    /// `inBufferSize` (`ByteCount`, 64-bit `unsigned long`) is the size of
    /// `buf` and `outActualSize` receives the parameter's real size.
    /// Safety: `event` is a live event owned by the runtime; `buf` must be
    /// valid for `buf_size` bytes (16 bytes for `hkid` + padding as
    /// `size_of::<EventHotKeyID>()`).
    pub fn GetEventParameter(
        event: EventRef,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: u64,
        actual_size: *mut u64,
        buf: *mut c_void,
    ) -> OSStatus;
}
