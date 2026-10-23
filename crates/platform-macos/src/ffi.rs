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
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    /// Release a CF object. Safety: `cf` must be a live owned CFTypeRef and
    /// must not be used afterwards.
    pub fn CFRelease(cf: CFTypeRef);

    /// Retain a CF object (bumps refcount). Safety: `cf` live CFTypeRef.
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;

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

    /// Compare two CF objects for equality. Safety: both must be live
    /// (non-null) CFTypeRefs.
    pub fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> Boolean;
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
