//! Safe macOS Accessibility adapter. All `unsafe` FFI lives in [`super::ffi`];
//! this module only composes safe wrappers with documented invariants.
//!
//! Scope (M1): one self-owned target window per pid for the single-window
//! commands. The multi-window path enumerates `AXWindows` for one pid
//! ([`MacAxAdapter::windows`]) and hands the caller a [`MacWindowSet`] whose
//! owned handles back a per-window runtime [`WindowKey`]. No enumeration of
//! user windows, no titles, no system-UI mutation, no workspaces, no private
//! APIs.

use std::fmt;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use layout_core::{
    DisplayId, DisplaySnapshot, PlatformSnapshot, Rect, WindowCapabilities, WindowConstraints,
    WindowId, WindowRole, WindowSnapshot,
};

use crate::batch::BatchOps;
use crate::ffi;

/// Process-lifetime CFString key. CFString constants are immutable and safe
/// to share across threads; the OnceLock writes each key exactly once.
#[derive(Debug, Clone, Copy)]
struct Key(ffi::CFStringRef);

// SAFETY: keys are immutable CFString constants created once at first use
// and never mutated or released; sharing the pointer across threads is safe.
unsafe impl Send for Key {}
unsafe impl Sync for Key {}

/// Process-lifetime CFString keys (SDK `CFSTR("...")` equivalents).
/// Built once via OnceLock and intentionally never released: they are
/// constants for the process duration, matching how the SDK macros behave.
static K_AX_WINDOWS: OnceLock<Key> = OnceLock::new();
static K_AX_MAIN_WINDOW: OnceLock<Key> = OnceLock::new();
static K_AX_POSITION: OnceLock<Key> = OnceLock::new();
static K_AX_SIZE: OnceLock<Key> = OnceLock::new();
static K_AX_RAISE: OnceLock<Key> = OnceLock::new();
static K_AX_FOCUSED_WINDOW: OnceLock<Key> = OnceLock::new();
static K_AX_SUBROLE: OnceLock<Key> = OnceLock::new();

fn k_ax_windows_attribute() -> ffi::CFStringRef {
    K_AX_WINDOWS
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXWindows\0")))
        .0
}

fn k_ax_main_window_attribute() -> ffi::CFStringRef {
    K_AX_MAIN_WINDOW
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXMainWindow\0")))
        .0
}

fn k_ax_position_attribute() -> ffi::CFStringRef {
    K_AX_POSITION
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXPosition\0")))
        .0
}

fn k_ax_size_attribute() -> ffi::CFStringRef {
    K_AX_SIZE
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXSize\0")))
        .0
}

fn k_ax_raise_action() -> ffi::CFStringRef {
    K_AX_RAISE
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXRaise\0")))
        .0
}

fn k_ax_focused_window_attribute() -> ffi::CFStringRef {
    K_AX_FOCUSED_WINDOW
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXFocusedWindow\0")))
        .0
}

fn k_ax_subrole_attribute() -> ffi::CFStringRef {
    K_AX_SUBROLE
        .get_or_init(|| Key(ffi::cf_string_from_static(b"AXSubrole\0")))
        .0
}

/// Typed failure modes. Every AXError code maps to exactly one variant so the
/// executor can branch on the real cause instead of a generic failure.
#[derive(Debug, Clone, PartialEq)]
pub enum AxError {
    /// Process lacks Accessibility permission (or AX API disabled).
    PermissionDenied,
    /// The target process/window no longer exists or its AX tree is gone.
    StaleTarget,
    /// Attribute or action not supported on this element/build.
    Unsupported,
    /// Illegal argument passed to an AX API.
    IllegalArgument,
    /// The application exposes no windows.
    NoWindows,
    /// Frame geometry is not writable: non-finite coordinate/size or
    /// non-positive size. Rejected before any write.
    InvalidGeometry,
    /// The position write was rejected before anything was applied. A full
    /// restore was still attempted and verified; `actual` is the best-known
    /// frame after the restore (None if it could not be read).
    PositionWriteFailed {
        source: Box<AxError>,
        rollback_restored: bool,
        actual: Option<Rect>,
    },
    /// The size write was rejected after the position write had already
    /// been applied. The saved geometry was restored (or the restore failed —
    /// `rollback_restored` says which); `actual` is the best-known frame after
    /// the restore (None if it could not be read).
    SizeAfterPositionFailed {
        source: Box<AxError>,
        rollback_restored: bool,
        actual: Option<Rect>,
    },
    /// Both writes were accepted, but the final read-back errored. A full
    /// restore was attempted; `actual` is the best-known frame after the
    /// restore (None if it could not be read).
    VerificationReadFailed {
        source: Box<AxError>,
        rollback_restored: bool,
        actual: Option<Rect>,
    },
    /// Both writes were accepted, but the read-back geometry did not match
    /// the request. The saved geometry was rolled back (or the rollback
    /// failed — see flag); `actual` reports the observed frame so a partial
    /// state is never hidden.
    VerificationFailed {
        expected: Rect,
        actual: Rect,
        rollback_restored: bool,
    },
    /// Any other AXError code, carried verbatim.
    Failure(i32),
}

impl fmt::Display for AxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => write!(f, "permission denied (Accessibility not granted)"),
            Self::StaleTarget => write!(f, "stale target (process/window gone)"),
            Self::Unsupported => write!(f, "unsupported attribute/action"),
            Self::IllegalArgument => write!(f, "illegal argument"),
            Self::NoWindows => write!(f, "no windows for target"),
            Self::InvalidGeometry => {
                write!(f, "invalid geometry (non-finite or non-positive size)")
            }
            Self::PositionWriteFailed {
                source,
                rollback_restored: true,
                ..
            } => write!(
                f,
                "position write rejected ({source}); original geometry restored"
            ),
            Self::PositionWriteFailed {
                source,
                rollback_restored: false,
                ..
            } => write!(
                f,
                "position write rejected ({source}); ROLLBACK FAILED - window state unverified"
            ),
            Self::SizeAfterPositionFailed {
                source,
                rollback_restored: true,
                ..
            } => write!(
                f,
                "size write rejected after position applied ({source}); original geometry restored"
            ),
            Self::SizeAfterPositionFailed {
                source,
                rollback_restored: false,
                ..
            } => write!(
                f,
                "size write rejected after position applied ({source}); ROLLBACK FAILED - window left at new position with old size"
            ),
            Self::VerificationReadFailed {
                source,
                rollback_restored: true,
                ..
            } => write!(
                f,
                "read-back failed after writes ({source}); original geometry restored"
            ),
            Self::VerificationReadFailed {
                source,
                rollback_restored: false,
                ..
            } => write!(
                f,
                "read-back failed after writes ({source}); ROLLBACK FAILED - window state unverified"
            ),
            Self::VerificationFailed {
                expected,
                actual,
                rollback_restored: true,
            } => write!(
                f,
                "read-back mismatch (expected {expected:?}, actual {actual:?}); original geometry restored"
            ),
            Self::VerificationFailed {
                expected,
                actual,
                rollback_restored: false,
            } => write!(
                f,
                "read-back mismatch (expected {expected:?}, actual {actual:?}); ROLLBACK FAILED - window left at unverified state"
            ),
            Self::Failure(code) => write!(f, "AX failure (code {code})"),
        }
    }
}

impl std::error::Error for AxError {}

/// Map a raw AXError code (AXError.h) to a typed error.
///
/// Codes verified against the host HIServices (macOS 26.7.1):
/// success 0 · failure -25200 · illegalArgument -25201 · invalidUIElement
/// -25202 · cannotComplete -25204 · attributeUnsupported -25205 ·
/// actionUnsupported -25206 · notificationUnsupported -25207 ·
/// notImplemented -25208 · notificationAlreadyRegistered -25209 ·
/// apiDisabled -25211 · noValue -25212 · parameterizedAttributeUnsupported
/// -25213 · notEnoughPrecision -25214.
pub fn map_ax_error(code: i32) -> AxError {
    match code {
        0 => unreachable!("map_ax_error called with success code"),
        -25211 => AxError::PermissionDenied,
        -25202 => AxError::StaleTarget,
        -25205 | -25206 | -25208 | -25212 | -25213 => AxError::Unsupported,
        -25201 => AxError::IllegalArgument,
        other => AxError::Failure(other),
    }
}

/// ESRCH — no such process (POSIX).
const K_ERRNO_ESRCH: i32 = 3;

/// Liveness from a `kill(pid, 0)` result plus errno.
///
/// Only ESRCH means the process is dead. EPERM means the process exists
/// but is owned by another user; any other errno also assumes "alive" so
/// the AX calls surface the real failure instead of a false stale report.
fn process_alive_from_kill(ret: i32, errno: i32) -> bool {
    if ret == 0 {
        return true;
    }
    errno != K_ERRNO_ESRCH
}

/// AXError codes that mean "the main-window attribute is not available"
/// and should fall back to the AXWindows list. Real failures (permission,
/// stale, cannotComplete, illegal argument) must propagate.
fn is_missing_main_window(code: i32) -> bool {
    matches!(code, -25212 | -25205) // noValue | attributeUnsupported
}

/// AX reports integer-ish points; allow a small tolerance when verifying
/// read-back geometry against a requested frame.
pub(crate) const K_FRAME_TOLERANCE: f64 = 1.0;

/// Frame comparison with tolerance.
pub(crate) fn frames_match(expected: Rect, actual: Rect, tolerance: f64) -> bool {
    (actual.x - expected.x).abs() <= tolerance
        && (actual.y - expected.y).abs() <= tolerance
        && (actual.width - expected.width).abs() <= tolerance
        && (actual.height - expected.height).abs() <= tolerance
}

/// Liveness check for `pid` (kill with sig 0 + errno: only ESRCH means
/// dead; EPERM means alive-but-not-ours). Exported for the probe binary,
/// which must not mutate a pid that was never registered as a fixture.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: kill with sig 0 sends no signal; standard POSIX check.
    // SAFETY: __error is the thread-local errno pointer (libSystem).
    let ret = unsafe { ffi::kill(pid as std::os::raw::c_int, 0) };
    let errno = unsafe { *ffi::__error() };
    process_alive_from_kill(ret, errno)
}

/// Validate frame geometry before any write, using the shared core
/// validator (finite coordinates, positive finite size, finite right/bottom).
fn validate_frame_rect(rect: Rect) -> Result<(), AxError> {
    if rect.is_finite_and_positive() {
        Ok(())
    } else {
        Err(AxError::InvalidGeometry)
    }
}

/// Injectable frame-operations seam. The real adapter implements this with
/// AX FFI; unit tests implement it with scripted outcomes (no Apple
/// frameworks) so the transaction contract can be verified without a Mac.
trait FrameOps {
    /// Read the current frame of the window owned by `el`.
    fn read_frame(&self, el: ffi::AXUIElementRef) -> Result<Rect, AxError>;
    /// Write the window position.
    fn write_position(&self, el: ffi::AXUIElementRef, x: f64, y: f64) -> Result<(), AxError>;
    /// Write the window size.
    fn write_size(&self, el: ffi::AXUIElementRef, w: f64, h: f64) -> Result<(), AxError>;
}

/// Full-frame rollback: restore position AND size to `before`, then verify by
/// reading back. Returns (whether the restore was verified, best-known frame
/// after the restore — None if it could not be read).
fn restore_full_frame<O: FrameOps>(
    ops: &O,
    el: ffi::AXUIElementRef,
    before: Rect,
) -> (bool, Option<Rect>) {
    let _ = ops.write_position(el, before.x, before.y);
    let _ = ops.write_size(el, before.width, before.height);
    match ops.read_frame(el) {
        Ok(r) => (frames_match(before, r, K_FRAME_TOLERANCE), Some(r)),
        Err(_) => (false, None),
    }
}

/// Transaction contract for a verified frame move/resize. Every post-mutation
/// failure attempts a full-frame rollback and reports the outcome; the caller
/// never sees a silent partial state. On success the verified frame is
/// returned so the CLI needs no extra (unhandled) post-transaction read.
fn set_window_frame_txn<O: FrameOps>(
    ops: &O,
    el: ffi::AXUIElementRef,
    rect: Rect,
) -> Result<Rect, AxError> {
    validate_frame_rect(rect)?;
    // Save first: if anything below fails, this is the rollback target.
    let before = ops.read_frame(el)?;

    if let Err(source) = ops.write_position(el, rect.x, rect.y) {
        // Position write rejected before anything was applied; still attempt a
        // full restore + verify so the reported state is never a guess.
        let (restored, actual) = restore_full_frame(ops, el, before);
        return Err(AxError::PositionWriteFailed {
            source: Box::new(source),
            rollback_restored: restored,
            actual,
        });
    }
    if let Err(source) = ops.write_size(el, rect.width, rect.height) {
        let (restored, actual) = restore_full_frame(ops, el, before);
        return Err(AxError::SizeAfterPositionFailed {
            source: Box::new(source),
            rollback_restored: restored,
            actual,
        });
    }
    // Both writes accepted: verify by read-back within tolerance.
    match ops.read_frame(el) {
        Ok(actual) => {
            if frames_match(rect, actual, K_FRAME_TOLERANCE) {
                return Ok(actual);
            }
            let (restored, _) = restore_full_frame(ops, el, before);
            Err(AxError::VerificationFailed {
                expected: rect,
                actual,
                rollback_restored: restored,
            })
        }
        Err(source) => {
            let (restored, actual) = restore_full_frame(ops, el, before);
            Err(AxError::VerificationReadFailed {
                source: Box::new(source),
                rollback_restored: restored,
                actual,
            })
        }
    }
}

/// Owned AXUIElement handle. Releases on drop.
///
/// Invariant: `element` is always a live AXUIElementRef or null; null is only
/// possible transiently inside fallible constructors.
#[derive(Debug)]
pub struct AxWindow {
    element: ffi::AXUIElementRef,
}

impl AxWindow {
    /// Take ownership of a raw element ref.
    ///
    /// Safety invariant for callers: `element` must be a live owned
    /// AXUIElementRef (e.g. from kAXMainWindowAttribute or a CFArray slot).
    /// A null pointer is stored as-is and treated as stale on first use.
    fn from_raw(element: ffi::AXUIElementRef) -> Self {
        Self { element }
    }

    pub(crate) fn element(&self) -> ffi::AXUIElementRef {
        self.element
    }
}

impl Drop for AxWindow {
    fn drop(&mut self) {
        if !self.element.is_null() {
            // SAFETY: element is a live owned AXUIElementRef; releasing once.
            unsafe { ffi::CFRelease(self.element as ffi::CFTypeRef) };
        }
    }
}

/// Stateless adapter. One instance per process; construct with
/// [`MacAxAdapter::connect`] which enforces the Accessibility permission gate.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacAxAdapter;

impl FrameOps for MacAxAdapter {
    fn read_frame(&self, el: ffi::AXUIElementRef) -> Result<Rect, AxError> {
        let (x, y) = self.copy_point(el, k_ax_position_attribute())?;
        let (w, h) = self.copy_size(el, k_ax_size_attribute())?;
        Ok(Rect::new(x, y, w, h))
    }

    fn write_position(&self, el: ffi::AXUIElementRef, x: f64, y: f64) -> Result<(), AxError> {
        self.write_point(el, k_ax_position_attribute(), x, y)
    }

    fn write_size(&self, el: ffi::AXUIElementRef, w: f64, h: f64) -> Result<(), AxError> {
        self.write_size(el, k_ax_size_attribute(), w, h)
    }
}

impl MacAxAdapter {
    /// Connect, failing with [`AxError::PermissionDenied`] when the process
    /// is not trusted for Accessibility.
    pub fn connect() -> Result<Self, AxError> {
        // SAFETY: AXIsProcessTrusted takes no arguments and touches no state.
        let trusted = unsafe { ffi::AXIsProcessTrusted() };
        if trusted == 0 {
            return Err(AxError::PermissionDenied);
        }
        Ok(Self)
    }

    /// Resolve the main window of the application owned by `pid`.
    ///
    /// Prefers kAXMainWindowAttribute; falls back to the first entry of
    /// kAXWindowsAttribute when the main-window attribute is unavailable
    /// (noValue / attributeUnsupported) or null. A dead pid surfaces as
    /// [`AxError::StaleTarget`] — the stale-handle path.
    pub fn main_window(&self, pid: u32) -> Result<AxWindow, AxError> {
        // SAFETY: pid is a valid process identifier; returns owned ref or null.
        let app = unsafe { ffi::AXUIElementCreateApplication(pid as std::os::raw::c_int) };
        if app.is_null() {
            return Err(AxError::StaleTarget);
        }
        // Stale pre-check: a dead pid surfaces as kAXErrorCannotComplete on
        // this build, so verify liveness explicitly (sig 0 = existence
        // only). Only ESRCH means dead; EPERM means alive-but-not-ours.
        // SAFETY: kill with sig 0 sends no signal; standard POSIX check.
        // SAFETY: __error is the thread-local errno pointer (libSystem).
        let ret = unsafe { ffi::kill(pid as std::os::raw::c_int, 0) };
        let errno = unsafe { *ffi::__error() };
        if !process_alive_from_kill(ret, errno) {
            // SAFETY: app is a live owned ref; released exactly once here.
            unsafe { ffi::CFRelease(app as ffi::CFTypeRef) };
            return Err(AxError::StaleTarget);
        }
        // SAFETY: app is a live application element; val receives an owned ref
        // that we release below (or transfer into AxWindow).
        let mut val: ffi::CFTypeRef = ptr::null();
        let code = unsafe {
            ffi::AXUIElementCopyAttributeValue(app, k_ax_main_window_attribute(), &mut val)
        };
        if code == 0 && !val.is_null() {
            // val (window) ownership transfers to AxWindow; the app element
            // is no longer needed and must be released on this success path.
            // SAFETY: app is a live owned ref; released exactly once here.
            unsafe { ffi::CFRelease(app as ffi::CFTypeRef) };
            return Ok(AxWindow::from_raw(val as ffi::AXUIElementRef));
        }
        if code != 0 && !is_missing_main_window(code) {
            // Real failure (permission, stale, cannotComplete, illegal):
            // propagate without fallback.
            let err = map_ax_error(code);
            // SAFETY: app is a live owned ref; released exactly once here.
            unsafe { ffi::CFRelease(app as ffi::CFTypeRef) };
            return Err(err);
        }
        // Attribute unavailable (noValue/attributeUnsupported) or null value:
        // fall back to the window list.
        // SAFETY: app live; list receives an owned CFArrayRef.
        let mut list: ffi::CFTypeRef = ptr::null();
        let code =
            unsafe { ffi::AXUIElementCopyAttributeValue(app, k_ax_windows_attribute(), &mut list) };
        // SAFETY: app is a live owned ref; released exactly once here.
        unsafe { ffi::CFRelease(app as ffi::CFTypeRef) };
        if code != 0 {
            return Err(map_ax_error(code));
        }
        if list.is_null() {
            return Err(AxError::NoWindows);
        }
        // SAFETY: list is a live owned CFArrayRef.
        let count = unsafe { ffi::CFArrayGetCount(list) };
        if count <= 0 {
            // SAFETY: list is a live owned CFArrayRef.
            unsafe { ffi::CFRelease(list) };
            return Err(AxError::NoWindows);
        }
        // SAFETY: list live, idx 0 in bounds; element is borrowed from list.
        let first = unsafe { ffi::CFArrayGetValueAtIndex(list, 0) } as ffi::AXUIElementRef;
        // SAFETY: list is a live owned CFArrayRef; the borrowed element stays
        // valid because we retain it by wrapping in AxWindow only after
        // retaining... CFArray does not transfer ownership, so we must
        // CFRetain the borrowed element before releasing the array.
        if first.is_null() {
            unsafe { ffi::CFRelease(list) };
            return Err(AxError::NoWindows);
        }
        // SAFETY: first is a borrowed live element from the array; CFRetain
        // gives us an owned reference independent of the array's lifetime.
        let retained = unsafe { ffi::CFRetain(first as ffi::CFTypeRef) };
        // SAFETY: list is a live owned CFArrayRef; released once here.
        unsafe { ffi::CFRelease(list) };
        Ok(AxWindow::from_raw(retained as ffi::AXUIElementRef))
    }

    /// Read the window frame in platform-neutral logical coordinates.
    pub fn window_frame(&self, window: &AxWindow) -> Result<Rect, AxError> {
        let (x, y) = self.copy_point(window.element(), k_ax_position_attribute())?;
        let (w, h) = self.copy_size(window.element(), k_ax_size_attribute())?;
        Ok(Rect::new(x, y, w, h))
    }

    /// Move and resize the window with a no-partial-mutation contract:
    ///
    /// 1. Preflight: geometry must be finite with positive size and finite
    ///    right/bottom ([`AxError::InvalidGeometry`] otherwise — nothing is
    ///    written).
    /// 2. The current geometry is saved before the first write.
    /// 3. Position is written, then size. If either write is rejected, the
    ///    saved geometry is fully restored (position + size) and verified.
    /// 4. After both writes are accepted, the frame is read back and compared
    ///    within tolerance. On read-back error or mismatch the saved geometry
    ///    is fully restored and verified.
    /// 5. Every post-mutation failure returns a typed error preserving the
    ///    original cause, the rollback status, and the best-known observed
    ///    frame — a partial state is never hidden.
    ///
    /// On success the verified frame is returned.
    pub fn set_window_frame(&self, window: &AxWindow, rect: Rect) -> Result<Rect, AxError> {
        set_window_frame_txn(self, window.element(), rect)
    }

    /// Request window raise (focus request). Explicit user-visible action;
    /// never called during passive discovery.
    pub fn raise(&self, window: &AxWindow) -> Result<(), AxError> {
        // SAFETY: window.element() is a live element; action is a valid key.
        let code = unsafe { ffi::AXUIElementPerformAction(window.element(), k_ax_raise_action()) };
        if code == 0 {
            Ok(())
        } else {
            Err(map_ax_error(code))
        }
    }

    fn copy_point(
        &self,
        element: ffi::AXUIElementRef,
        attr: ffi::CFStringRef,
    ) -> Result<(f64, f64), AxError> {
        let value = self.copy_value(element, attr)?;
        // SAFETY: value is a live AXValue of point type; out buffer is 2×f64.
        let mut point = CGPoint { x: 0.0, y: 0.0 };
        let ok = unsafe {
            ffi::AXValueGetValue(
                value,
                ffi::K_AX_VALUE_CG_POINT_TYPE,
                &mut point as *mut CGPoint as *mut _,
            )
        };
        // SAFETY: value is a live owned AXValueRef; released once here.
        unsafe { ffi::CFRelease(value) };
        if ok == 0 {
            return Err(AxError::Failure(-25214)); // notEnoughPrecision
        }
        Ok((point.x, point.y))
    }

    fn copy_size(
        &self,
        element: ffi::AXUIElementRef,
        attr: ffi::CFStringRef,
    ) -> Result<(f64, f64), AxError> {
        let value = self.copy_value(element, attr)?;
        // SAFETY: value is a live AXValue of size type; out buffer is 2×f64.
        let mut size = CGSize {
            width: 0.0,
            height: 0.0,
        };
        let ok = unsafe {
            ffi::AXValueGetValue(
                value,
                ffi::K_AX_VALUE_CG_SIZE_TYPE,
                &mut size as *mut CGSize as *mut _,
            )
        };
        // SAFETY: value is a live owned AXValueRef; released once here.
        unsafe { ffi::CFRelease(value) };
        if ok == 0 {
            return Err(AxError::Failure(-25214));
        }
        Ok((size.width, size.height))
    }

    fn copy_value(
        &self,
        element: ffi::AXUIElementRef,
        attr: ffi::CFStringRef,
    ) -> Result<ffi::AXValueRef, AxError> {
        // SAFETY: element is live; value receives an owned CFTypeRef.
        let mut value: ffi::CFTypeRef = ptr::null();
        let code = unsafe { ffi::AXUIElementCopyAttributeValue(element, attr, &mut value) };
        if code != 0 {
            return Err(map_ax_error(code));
        }
        if value.is_null() {
            return Err(AxError::Unsupported);
        }
        Ok(value as ffi::AXValueRef)
    }

    fn write_point(
        &self,
        element: ffi::AXUIElementRef,
        attr: ffi::CFStringRef,
        x: f64,
        y: f64,
    ) -> Result<(), AxError> {
        let point = CGPoint { x, y };
        // SAFETY: point matches K_AX_VALUE_CG_POINT_TYPE; returns owned ref.
        let value = unsafe {
            ffi::AXValueCreate(
                ffi::K_AX_VALUE_CG_POINT_TYPE,
                &point as *const CGPoint as *const _,
            )
        };
        if value.is_null() {
            return Err(AxError::Failure(-25200));
        }
        // SAFETY: element live; value is a borrowed AXValue for the call.
        let code = unsafe { ffi::AXUIElementSetAttributeValue(element, attr, value) };
        // SAFETY: value is a live owned AXValueRef; released once here.
        unsafe { ffi::CFRelease(value) };
        if code == 0 {
            Ok(())
        } else {
            Err(map_ax_error(code))
        }
    }

    fn write_size(
        &self,
        element: ffi::AXUIElementRef,
        attr: ffi::CFStringRef,
        width: f64,
        height: f64,
    ) -> Result<(), AxError> {
        let size = CGSize { width, height };
        // SAFETY: size matches K_AX_VALUE_CG_SIZE_TYPE; returns owned ref.
        let value = unsafe {
            ffi::AXValueCreate(
                ffi::K_AX_VALUE_CG_SIZE_TYPE,
                &size as *const CGSize as *const _,
            )
        };
        if value.is_null() {
            return Err(AxError::Failure(-25200));
        }
        // SAFETY: element live; value is a borrowed AXValue for the call.
        let code = unsafe { ffi::AXUIElementSetAttributeValue(element, attr, value) };
        // SAFETY: value is a live owned AXValueRef; released once here.
        unsafe { ffi::CFRelease(value) };
        if code == 0 {
            Ok(())
        } else {
            Err(map_ax_error(code))
        }
    }
}

/// Owned CoreFoundation object released on drop. Used for the transient
/// `AXWindows` array and focused-window element; [`AxWindow`] owns elements
/// that must outlive the call.
struct CfOwned(ffi::CFTypeRef);

impl Drop for CfOwned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 is a live owned CFTypeRef released exactly once.
            unsafe { ffi::CFRelease(self.0) };
        }
    }
}

/// Opaque runtime identity for one window within one observation.
///
/// Generated by the adapter while enumerating a process's windows and valid
/// only for the [`MacWindowSet`] that produced it: it is not a native handle,
/// not derived from a window title, and must never be persisted or carried
/// across observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowKey(pub u64);

/// Runtime map from planner window IDs to native window keys for one
/// observation.
///
/// The domain never sees a [`WindowKey`]; this map is the only seam between the
/// two identities, and it is valid only for the observation that built it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowIdMap {
    pairs: Vec<(WindowId, WindowKey)>,
}

impl WindowIdMap {
    /// Native key for a planner window ID, if this observation observed it.
    pub fn get(&self, id: WindowId) -> Option<WindowKey> {
        self.pairs
            .iter()
            .find(|(mapped, _)| *mapped == id)
            .map(|(_, key)| *key)
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// One observation: the platform-neutral snapshot the pure planner consumes
/// plus the ID map the native executor needs.
#[derive(Debug, Clone, PartialEq)]
pub struct MacObservation {
    pub snapshot: PlatformSnapshot,
    pub map: WindowIdMap,
}

/// Monotonic observation revision. Adapter-local; never persisted.
static OBSERVATION_REVISION: AtomicU64 = AtomicU64::new(1);

/// Map one window to its planner role before user rules apply.
///
/// Mandatory exclusions by identity (checked first, not overridable by any
/// config value; the planner always excludes them):
/// - the pid equals this controller's own pid ([`WindowRole::OwnApplication`]).
///
/// Policy classification from the Accessibility subrole (the planner excludes
/// these only when the plan's dialog policy says so):
/// - dialog/floating subroles → [`WindowRole::Dialog`]: the canonical dialog
///   subroles (`AXDialogSubrole`, `AXSystemDialogSubrole`,
///   `AXFloatingWindowSubrole`) and `AXSystemFloatingWindow` (runtime-observed
///   on Tahoe 26.7.1 for a floating utility panel via the Swift probe script,
///   not assumed from documentation).
/// - notification surfaces → [`WindowRole::System`]: `AXNotificationCenterAlert`,
///   `AXNotificationCenterBanner`.
///
/// A missing or unrecognized subrole (including the ordinary
/// `AXStandardWindow` observed for plain `NSWindow`s) classifies as an
/// ordinary [`WindowRole::Application`]. This is the documented policy
/// surface, not a mandatory gate: mandatory exclusions rest on identity
/// (own pid) and on the write-time verified-write-plus-rollback contract.
pub fn classify_window(pid_is_own: bool, subrole: Option<&str>) -> WindowRole {
    if pid_is_own {
        return WindowRole::OwnApplication;
    }
    match subrole {
        Some(
            "AXDialogSubrole"
            | "AXSystemDialogSubrole"
            | "AXFloatingWindowSubrole"
            | "AXSystemFloatingWindow",
        ) => WindowRole::Dialog,
        Some("AXNotificationCenterAlert" | "AXNotificationCenterBanner") => WindowRole::System,
        _ => WindowRole::Application,
    }
}

/// Read a window's `AXSubrole` string, or `None` when the attribute is
/// unavailable (unsupported or no value is normal for ordinary windows).
fn window_subrole(element: &AxWindow) -> Option<String> {
    // SAFETY: element is live; the copy returns an owned ref or null.
    let mut value: ffi::CFTypeRef = ptr::null();
    let code = unsafe {
        ffi::AXUIElementCopyAttributeValue(element.element(), k_ax_subrole_attribute(), &mut value)
    };
    if code != 0 || value.is_null() {
        return None;
    }
    // SAFETY: value is an owned ref or null; the guard releases it.
    let owned = CfOwned(value);
    let mut buffer = [0u8; 64];
    // SAFETY: owned.0 is a live CFStringRef and buffer meets capacity + the
    // documented encoding.
    let ok = unsafe {
        ffi::CFStringGetCString(
            owned.0 as ffi::CFStringRef,
            buffer.as_mut_ptr() as *mut i8,
            buffer.len() as isize,
            ffi::K_CF_STRING_ENCODING_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(0);
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

/// Build a snapshot and ID map from enumerated windows. Pure: it makes no OS
/// calls, so the planner/executor seam is unit-testable with several windows
/// from one process.
///
/// `display` is the usable rectangle supplied by the caller; the adapter does
/// not query monitors yet, so `bounds == work_area` and the scale factor is
/// `1.0` (logical points, single display — a documented POC limit).
///
/// Capability bits are optimistic here. The M3 review established that
/// `AXUIElementIsAttributeSettable` returning false for `AXSize` is not a
/// reliable exclusion (it can disagree with a verified write), so settable is
/// not used as the eligibility gate; the real gate is the verified write plus
/// rollback in [`MacAxAdapter::set_window_frame`] and
/// [`crate::batch::apply_batch`]. Every entry carries a
/// [`WindowRole`] from [`classify_window`]: the planner excludes
/// `OwnApplication`/`System`/`Desktop` unconditionally and dialogs per the
/// plan's dialog policy.
pub fn observation_from_windows(
    display: Rect,
    windows: &[(WindowKey, Rect, WindowRole)],
    focused: Option<WindowKey>,
) -> MacObservation {
    let display_id = DisplayId(0);
    let mut snapshots = Vec::with_capacity(windows.len());
    let mut map = WindowIdMap::default();
    for (key, frame, role) in windows {
        let id = WindowId(key.0);
        map.pairs.push((id, *key));
        snapshots.push(WindowSnapshot {
            id,
            display: display_id,
            frame: *frame,
            constraints: WindowConstraints::UNBOUNDED,
            role: *role,
            capabilities: WindowCapabilities {
                can_move: true,
                can_resize: true,
                can_focus: true,
            },
        });
    }
    let focused_window = focused
        .filter(|key| windows.iter().any(|(observed, _, _)| observed == key))
        .map(|key| WindowId(key.0));
    MacObservation {
        snapshot: PlatformSnapshot {
            revision: OBSERVATION_REVISION.fetch_add(1, Ordering::Relaxed),
            displays: vec![DisplaySnapshot {
                id: display_id,
                bounds: display,
                work_area: display,
                scale_factor: 1.0,
            }],
            windows: snapshots,
            focused_window,
        },
        map,
    }
}

/// One enumerated window: runtime key, owned AX handle, the frame read at
/// enumeration time, and the classification from [`classify_window`].
#[derive(Debug)]
struct MacWindowEntry {
    key: WindowKey,
    handle: AxWindow,
    frame: Rect,
    role: WindowRole,
}

/// A resolved set of windows for one process, owning the AX element handles.
///
/// The set is the executor's view of a process: [`crate::batch::apply_batch`]
/// looks keys up in these owned handles, so a key is never re-resolved against
/// a window list that changed after enumeration.
#[derive(Debug)]
pub struct MacWindowSet {
    adapter: MacAxAdapter,
    pid: u32,
    windows: Vec<MacWindowEntry>,
    focused: Option<WindowKey>,
}

impl MacWindowSet {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Whether `key` was observed in this set.
    pub fn contains(&self, key: WindowKey) -> bool {
        self.windows.iter().any(|entry| entry.key == key)
    }

    /// `(key, frame, role)` for every window, in adapter order.
    pub fn entries(&self) -> impl Iterator<Item = (WindowKey, Rect, WindowRole)> + '_ {
        self.windows
            .iter()
            .map(|entry| (entry.key, entry.frame, entry.role))
    }

    /// Snapshot this set against `display` for the pure planner, plus the ID
    /// map the executor needs to turn placements back into native targets.
    pub fn observe(&self, display: Rect) -> MacObservation {
        let entries: Vec<(WindowKey, Rect, WindowRole)> = self.entries().collect();
        observation_from_windows(display, &entries, self.focused)
    }

    fn handle(&self, window: WindowKey) -> Result<&AxWindow, AxError> {
        self.windows
            .iter()
            .find(|entry| entry.key == window)
            .map(|entry| &entry.handle)
            .ok_or(AxError::StaleTarget)
    }
}

impl BatchOps for MacWindowSet {
    fn read_frame(&self, window: WindowKey) -> Result<Rect, AxError> {
        self.adapter.window_frame(self.handle(window)?)
    }

    fn set_frame(&self, window: WindowKey, rect: Rect) -> Result<Rect, AxError> {
        self.adapter.set_window_frame(self.handle(window)?, rect)
    }

    fn restore(&self, window: WindowKey, before: Rect) -> (bool, Option<Rect>) {
        match self.set_frame(window, before) {
            Ok(actual) => (
                frames_match(before, actual, K_FRAME_TOLERANCE),
                Some(actual),
            ),
            Err(_) => (false, None),
        }
    }
}

impl MacAxAdapter {
    /// Enumerate the windows of the process owned by `pid`.
    ///
    /// Reads `kAXWindowsAttribute`, checks each element's owning pid
    /// (`AXUIElementGetPid`), reads its frame and its `AXSubrole`
    /// classification ([`classify_window`]), and records which window
    /// (`kAXFocusedWindowAttribute`) is focused. Keys are assigned in list
    /// order. Fails closed: a dead pid is [`AxError::StaleTarget`], a process
    /// with no windows is [`AxError::NoWindows`], and a foreign element or an
    /// unreadable frame propagates instead of being silently dropped.
    pub fn windows(&self, pid: u32) -> Result<MacWindowSet, AxError> {
        // SAFETY: pid is a valid process identifier; returns owned ref or null.
        let app = unsafe { ffi::AXUIElementCreateApplication(pid as std::os::raw::c_int) };
        if app.is_null() {
            return Err(AxError::StaleTarget);
        }
        // SAFETY: app is a live owned ref; the guard releases it exactly once.
        let app = CfOwned(app as ffi::CFTypeRef);
        if !process_alive(pid) {
            return Err(AxError::StaleTarget);
        }
        // The focused window is optional: noValue/attributeUnsupported just
        // means the app reports no focused window, which is not a failure.
        // SAFETY: app is live; the copy returns an owned ref or null.
        let mut focused_val: ffi::CFTypeRef = ptr::null();
        let focused_code = unsafe {
            ffi::AXUIElementCopyAttributeValue(
                app.0,
                k_ax_focused_window_attribute(),
                &mut focused_val,
            )
        };
        if focused_code != 0 {
            focused_val = ptr::null();
        }
        // SAFETY: focused_val is an owned ref or null; the guard releases it.
        let focused = CfOwned(focused_val);
        // SAFETY: app is live; list receives an owned CFArrayRef or null.
        let mut list: ffi::CFTypeRef = ptr::null();
        let code = unsafe {
            ffi::AXUIElementCopyAttributeValue(app.0, k_ax_windows_attribute(), &mut list)
        };
        if code != 0 {
            return Err(map_ax_error(code));
        }
        // SAFETY: list is an owned CFArrayRef or null; the guard releases it.
        let list = CfOwned(list);
        if list.0.is_null() {
            return Err(AxError::NoWindows);
        }
        // SAFETY: list is a live owned CFArrayRef.
        let count = unsafe { ffi::CFArrayGetCount(list.0) };
        if count <= 0 {
            return Err(AxError::NoWindows);
        }
        let mut windows = Vec::with_capacity(count as usize);
        let mut focused_key = None;
        for index in 0..count {
            // SAFETY: list live, index in bounds; element is borrowed.
            let raw = unsafe { ffi::CFArrayGetValueAtIndex(list.0, index) } as ffi::AXUIElementRef;
            if raw.is_null() {
                continue;
            }
            // Fail closed on an element that does not belong to `pid`.
            let mut element_pid: std::os::raw::c_int = 0;
            // SAFETY: raw is a live borrowed element; element_pid is a pid_t.
            let pid_code = unsafe { ffi::AXUIElementGetPid(raw, &mut element_pid) };
            if pid_code != 0 || element_pid as u32 != pid {
                return Err(AxError::StaleTarget);
            }
            // SAFETY: raw is borrowed from the array; CFRetain gives an owned
            // reference independent of the array's lifetime.
            let retained = unsafe { ffi::CFRetain(raw as ffi::CFTypeRef) } as ffi::AXUIElementRef;
            let handle = AxWindow::from_raw(retained);
            let frame = self.window_frame(&handle)?;
            let subrole = window_subrole(&handle);
            let role = classify_window(pid == std::process::id(), subrole.as_deref());
            let key = WindowKey(windows.len() as u64);
            if !focused.0.is_null() {
                // SAFETY: raw and focused.0 are live CFTypeRefs.
                let equal = unsafe { ffi::CFEqual(raw as ffi::CFTypeRef, focused.0) };
                if equal != 0 {
                    focused_key = Some(key);
                }
            }
            windows.push(MacWindowEntry {
                key,
                handle,
                frame,
                role,
            });
        }
        if windows.is_empty() {
            return Err(AxError::NoWindows);
        }
        Ok(MacWindowSet {
            adapter: *self,
            pid,
            windows,
            focused: focused_key,
        })
    }
}

/// CGPoint layout (CoreGraphics): two CGFloats. On x86_64 CGFloat = f64.
#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

/// CGSize layout (CoreGraphics): two CGFloats. On x86_64 CGFloat = f64.
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn ax_error_codes_map_to_distinct_variants() {
        assert_eq!(map_ax_error(-25211), AxError::PermissionDenied);
        assert_eq!(map_ax_error(-25202), AxError::StaleTarget);
        assert_eq!(map_ax_error(-25205), AxError::Unsupported);
        assert_eq!(map_ax_error(-25206), AxError::Unsupported);
        assert_eq!(map_ax_error(-25208), AxError::Unsupported);
        assert_eq!(map_ax_error(-25212), AxError::Unsupported);
        assert_eq!(map_ax_error(-25213), AxError::Unsupported);
        assert_eq!(map_ax_error(-25201), AxError::IllegalArgument);
        assert_eq!(map_ax_error(-25200), AxError::Failure(-25200));
        assert_eq!(map_ax_error(-25204), AxError::Failure(-25204));
        assert_eq!(map_ax_error(-25207), AxError::Failure(-25207));
        assert_eq!(map_ax_error(-25209), AxError::Failure(-25209));
        assert_eq!(map_ax_error(-25210), AxError::Failure(-25210));
        assert_eq!(map_ax_error(-25214), AxError::Failure(-25214));
    }

    #[test]
    fn error_display_is_human_readable() {
        assert!(AxError::PermissionDenied.to_string().contains("permission"));
        assert!(AxError::StaleTarget.to_string().contains("stale"));
        assert!(AxError::Unsupported.to_string().contains("unsupported"));
        assert!(AxError::NoWindows.to_string().contains("no windows"));
        assert!(AxError::Failure(-25204).to_string().contains("-25204"));
        assert!(AxError::InvalidGeometry
            .to_string()
            .contains("invalid geometry"));
        let rolled_back = AxError::SizeAfterPositionFailed {
            source: Box::new(AxError::Unsupported),
            rollback_restored: true,
            actual: None,
        };
        assert!(rolled_back
            .to_string()
            .contains("original geometry restored"));
        let not_restored = AxError::SizeAfterPositionFailed {
            source: Box::new(AxError::Unsupported),
            rollback_restored: false,
            actual: None,
        };
        assert!(not_restored.to_string().contains("ROLLBACK FAILED"));
    }

    #[test]
    fn liveness_check_distinguishes_esrch_from_eperm() {
        // ret 0 = alive; ESRCH = dead; EPERM/other = alive (AX will
        // surface the real error).
        assert!(process_alive_from_kill(0, 0));
        assert!(!process_alive_from_kill(-1, K_ERRNO_ESRCH));
        assert!(process_alive_from_kill(-1, 1)); // EPERM: exists, not ours
        assert!(process_alive_from_kill(-1, 22)); // EINVAL: assume alive
    }

    #[test]
    fn main_window_fallback_only_for_missing_attribute() {
        // noValue / attributeUnsupported -> fallback to AXWindows.
        assert!(is_missing_main_window(-25212));
        assert!(is_missing_main_window(-25205));
        // Real failures propagate.
        assert!(!is_missing_main_window(-25211)); // apiDisabled
        assert!(!is_missing_main_window(-25202)); // invalidUIElement
        assert!(!is_missing_main_window(-25204)); // cannotComplete
        assert!(!is_missing_main_window(-25201)); // illegalArgument
    }

    #[test]
    fn frame_validation_rejects_non_finite_and_non_positive() {
        assert!(validate_frame_rect(Rect::new(0.0, 0.0, 100.0, 50.0)).is_ok());
        assert_eq!(
            validate_frame_rect(Rect::new(f64::NAN, 0.0, 100.0, 50.0)),
            Err(AxError::InvalidGeometry)
        );
        assert_eq!(
            validate_frame_rect(Rect::new(0.0, f64::INFINITY, 100.0, 50.0)),
            Err(AxError::InvalidGeometry)
        );
        assert_eq!(
            validate_frame_rect(Rect::new(0.0, 0.0, 0.0, 50.0)),
            Err(AxError::InvalidGeometry)
        );
        assert_eq!(
            validate_frame_rect(Rect::new(0.0, 0.0, 100.0, -1.0)),
            Err(AxError::InvalidGeometry)
        );
    }

    #[test]
    fn frames_match_uses_tolerance() {
        let expected = Rect::new(100.0, 668.0, 420.0, 312.0);
        assert!(frames_match(expected, expected, 1.0));
        assert!(frames_match(
            expected,
            Rect::new(100.5, 668.0, 419.5, 312.0),
            1.0
        ));
        assert!(!frames_match(
            expected,
            Rect::new(102.0, 668.0, 420.0, 312.0),
            1.0
        ));
        assert!(!frames_match(
            expected,
            Rect::new(100.0, 668.0, 420.0, 315.0),
            1.0
        ));
    }

    #[test]
    fn classify_window_maps_subroles_and_own_pid() {
        // The mandatory identity gate wins over every subrole.
        assert_eq!(
            classify_window(true, Some("AXDialogSubrole")),
            WindowRole::OwnApplication
        );
        // Known dialog subroles are policy-excludable, not mandatory.
        for subrole in [
            "AXDialogSubrole",
            "AXSystemDialogSubrole",
            "AXFloatingWindowSubrole",
            "AXSystemFloatingWindow",
        ] {
            assert_eq!(classify_window(false, Some(subrole)), WindowRole::Dialog);
        }
        // Notification surfaces are system UI: always excluded by the planner.
        for subrole in ["AXNotificationCenterAlert", "AXNotificationCenterBanner"] {
            assert_eq!(classify_window(false, Some(subrole)), WindowRole::System);
        }
        // A missing, ordinary or unknown subrole stays an application window.
        assert_eq!(classify_window(false, None), WindowRole::Application);
        assert_eq!(
            classify_window(false, Some("AXStandardWindow")),
            WindowRole::Application
        );
        assert_eq!(
            classify_window(false, Some("AXUnknownSubrole")),
            WindowRole::Application
        );
    }

    #[test]
    fn observation_carries_roles_into_the_snapshot() {
        let observation = observation_from_windows(
            Rect::new(0.0, 0.0, 1200.0, 800.0),
            &[
                (
                    WindowKey(0),
                    Rect::new(0.0, 0.0, 100.0, 100.0),
                    WindowRole::Application,
                ),
                (
                    WindowKey(1),
                    Rect::new(100.0, 0.0, 100.0, 100.0),
                    WindowRole::Dialog,
                ),
                (
                    WindowKey(2),
                    Rect::new(0.0, 0.0, 50.0, 50.0),
                    WindowRole::OwnApplication,
                ),
            ],
            Some(WindowKey(0)),
        );
        let roles: Vec<WindowRole> = observation
            .snapshot
            .windows
            .iter()
            .map(|window| window.role)
            .collect();
        assert_eq!(
            roles,
            vec![
                WindowRole::Application,
                WindowRole::Dialog,
                WindowRole::OwnApplication
            ]
        );
        // The planner excludes OwnApplication and (by default) Dialog, so a
        // plan over this observation tiles exactly one window.
        let plan = layout_core::plan_snapshot(
            &observation.snapshot,
            DisplayId(0),
            layout_core::LayoutKind::MasterStack,
            layout_core::LayoutConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(plan.placements.len(), 1);
        let excluded_reasons: Vec<_> = plan.excluded.iter().map(|item| item.reason).collect();
        assert!(excluded_reasons.contains(&layout_core::ExclusionReason::OwnApplication));
        assert!(excluded_reasons.contains(&layout_core::ExclusionReason::DialogPolicy));
    }

    // MARK: - Transaction contract (mock FrameOps; no Apple frameworks)

    /// Scripted mock of [`FrameOps`] for unit tests. The first read is the
    /// save; subsequent reads are the read-back (and rollback verify). Write
    /// failures are controlled per-scenario; `fail_writes_after` fails any
    /// write once that many writes have succeeded (to simulate rollback-write
    /// failure).
    struct MockOps {
        frame: Cell<Rect>,
        read_override: Option<Rect>,
        fail_read_back: bool,
        fail_rollback_read: bool,
        fail_position: bool,
        fail_size: bool,
        fail_writes_after: Option<usize>,
        writes: std::cell::RefCell<Vec<String>>,
        reads: Cell<usize>,
        position_failed: Cell<bool>,
        size_failed: Cell<bool>,
    }

    impl MockOps {
        fn new(frame: Rect) -> Self {
            Self {
                frame: Cell::new(frame),
                read_override: None,
                fail_read_back: false,
                fail_rollback_read: false,
                fail_position: false,
                fail_size: false,
                fail_writes_after: None,
                writes: std::cell::RefCell::new(Vec::new()),
                reads: Cell::new(0),
                position_failed: Cell::new(false),
                size_failed: Cell::new(false),
            }
        }

        fn write_log(&self) -> Vec<String> {
            self.writes.borrow().clone()
        }
    }

    impl FrameOps for MockOps {
        fn read_frame(&self, _el: ffi::AXUIElementRef) -> Result<Rect, AxError> {
            let n = self.reads.get();
            self.reads.set(n + 1);
            if n == 1 {
                // First read-back.
                if self.fail_read_back {
                    return Err(AxError::Failure(-25204));
                }
                if let Some(r) = self.read_override {
                    return Ok(r);
                }
            }
            if n >= 2 && self.fail_rollback_read {
                // Rollback verify read.
                return Err(AxError::Failure(-25204));
            }
            Ok(self.frame.get())
        }

        fn write_position(&self, _el: ffi::AXUIElementRef, x: f64, y: f64) -> Result<(), AxError> {
            if self.fail_position && !self.position_failed.get() {
                self.position_failed.set(true);
                return Err(AxError::Failure(-25200));
            }
            if self
                .fail_writes_after
                .is_some_and(|t| self.writes.borrow().len() >= t)
            {
                return Err(AxError::Failure(-25200));
            }
            self.writes.borrow_mut().push("position".to_string());
            let f = self.frame.get();
            self.frame.set(Rect::new(x, y, f.width, f.height));
            Ok(())
        }

        fn write_size(&self, _el: ffi::AXUIElementRef, w: f64, h: f64) -> Result<(), AxError> {
            if self.fail_size && !self.size_failed.get() {
                self.size_failed.set(true);
                return Err(AxError::Failure(-25200));
            }
            if self
                .fail_writes_after
                .is_some_and(|t| self.writes.borrow().len() >= t)
            {
                return Err(AxError::Failure(-25200));
            }
            self.writes.borrow_mut().push("size".to_string());
            let f = self.frame.get();
            self.frame.set(Rect::new(f.x, f.y, w, h));
            Ok(())
        }
    }

    const EL: ffi::AXUIElementRef = ptr::null();

    #[test]
    fn invalid_geometry_causes_zero_writes() {
        let ops = MockOps::new(Rect::new(0.0, 0.0, 100.0, 50.0));
        let rect = Rect::new(f64::NAN, 0.0, 100.0, 50.0);
        let err = set_window_frame_txn(&ops, EL, rect).unwrap_err();
        assert_eq!(err, AxError::InvalidGeometry);
        assert!(ops.write_log().is_empty());
        // Zero-size also rejected before any write.
        let err = set_window_frame_txn(&ops, EL, Rect::new(0.0, 0.0, 0.0, 50.0)).unwrap_err();
        assert_eq!(err, AxError::InvalidGeometry);
        assert!(ops.write_log().is_empty());
    }

    #[test]
    fn success_returns_verified_frame() {
        let target = Rect::new(120.0, 120.0, 420.0, 312.0);
        let ops = MockOps::new(Rect::new(100.0, 668.0, 420.0, 312.0));
        let verified = set_window_frame_txn(&ops, EL, target).unwrap();
        assert_eq!(verified, target);
        assert_eq!(ops.write_log(), vec!["position", "size"]);
    }

    #[test]
    fn position_write_failure_attempts_full_rollback() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let mut ops = MockOps::new(before);
        ops.fail_position = true;
        let err =
            set_window_frame_txn(&ops, EL, Rect::new(120.0, 120.0, 420.0, 312.0)).unwrap_err();
        match err {
            AxError::PositionWriteFailed {
                source,
                rollback_restored,
                actual,
            } => {
                assert_eq!(*source, AxError::Failure(-25200));
                assert!(rollback_restored);
                assert_eq!(actual, Some(before));
            }
            other => panic!("expected PositionWriteFailed, got {other:?}"),
        }
        // The failed position write is not logged; rollback restores position + size.
        assert_eq!(ops.write_log(), vec!["position", "size"]);
    }

    #[test]
    fn size_write_failure_attempts_full_rollback() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let mut ops = MockOps::new(before);
        ops.fail_size = true;
        let err =
            set_window_frame_txn(&ops, EL, Rect::new(120.0, 120.0, 420.0, 312.0)).unwrap_err();
        match err {
            AxError::SizeAfterPositionFailed {
                source,
                rollback_restored,
                actual,
            } => {
                assert_eq!(*source, AxError::Failure(-25200));
                assert!(rollback_restored);
                assert_eq!(actual, Some(before));
            }
            other => panic!("expected SizeAfterPositionFailed, got {other:?}"),
        }
        // position applied, size failed, rollback restores position + size.
        assert_eq!(ops.write_log(), vec!["position", "position", "size"]);
    }

    #[test]
    fn read_back_error_attempts_full_rollback() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let mut ops = MockOps::new(before);
        ops.fail_read_back = true;
        let err =
            set_window_frame_txn(&ops, EL, Rect::new(120.0, 120.0, 420.0, 312.0)).unwrap_err();
        match err {
            AxError::VerificationReadFailed {
                source,
                rollback_restored,
                actual,
            } => {
                assert_eq!(*source, AxError::Failure(-25204));
                assert!(rollback_restored);
                assert_eq!(actual, Some(before));
            }
            other => panic!("expected VerificationReadFailed, got {other:?}"),
        }
        // writes: position, size (applied) + rollback position, size.
        assert_eq!(
            ops.write_log(),
            vec!["position", "size", "position", "size"]
        );
    }

    #[test]
    fn read_back_mismatch_attempts_full_rollback() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let target = Rect::new(120.0, 120.0, 420.0, 312.0);
        let mut ops = MockOps::new(before);
        // Read-back reports a frame that does not match the request.
        ops.read_override = Some(Rect::new(999.0, 999.0, 420.0, 312.0));
        let err = set_window_frame_txn(&ops, EL, target).unwrap_err();
        match err {
            AxError::VerificationFailed {
                expected,
                actual,
                rollback_restored,
            } => {
                assert_eq!(expected, target);
                assert_eq!(actual, Rect::new(999.0, 999.0, 420.0, 312.0));
                assert!(rollback_restored);
            }
            other => panic!("expected VerificationFailed, got {other:?}"),
        }
        // writes: position, size (applied) + rollback position, size.
        assert_eq!(
            ops.write_log(),
            vec!["position", "size", "position", "size"]
        );
    }

    #[test]
    fn rollback_write_failure_is_reported_not_restored() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let target = Rect::new(120.0, 120.0, 420.0, 312.0);
        let mut ops = MockOps::new(before);
        // Read-back mismatches, then the rollback writes fail.
        ops.read_override = Some(Rect::new(999.0, 999.0, 420.0, 312.0));
        ops.fail_writes_after = Some(2); // fail writes once 2 have succeeded
        let err = set_window_frame_txn(&ops, EL, target).unwrap_err();
        match err {
            AxError::VerificationFailed {
                rollback_restored,
                actual,
                ..
            } => {
                assert!(!rollback_restored);
                // actual is the observed mismatch (not the post-rollback frame).
                assert_eq!(actual, Rect::new(999.0, 999.0, 420.0, 312.0));
            }
            other => panic!("expected VerificationFailed, got {other:?}"),
        }
    }

    #[test]
    fn rollback_read_failure_is_reported_not_restored() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let target = Rect::new(120.0, 120.0, 420.0, 312.0);
        let mut ops = MockOps::new(before);
        // Read-back mismatches; rollback writes succeed but the verify read
        // fails (best-known frame = None).
        ops.read_override = Some(Rect::new(999.0, 999.0, 420.0, 312.0));
        ops.fail_rollback_read = true;
        let err = set_window_frame_txn(&ops, EL, target).unwrap_err();
        match err {
            AxError::VerificationFailed {
                rollback_restored,
                actual,
                ..
            } => {
                assert!(!rollback_restored);
                assert_eq!(actual, Rect::new(999.0, 999.0, 420.0, 312.0));
            }
            other => panic!("expected VerificationFailed, got {other:?}"),
        }
    }

    #[test]
    fn tolerance_boundary_behavior() {
        let target = Rect::new(120.0, 120.0, 420.0, 312.0);
        // Exactly at tolerance -> match.
        let mut ops = MockOps::new(Rect::new(100.0, 668.0, 420.0, 312.0));
        ops.read_override = Some(Rect::new(121.0, 120.0, 420.0, 312.0));
        let verified = set_window_frame_txn(&ops, EL, target).unwrap();
        assert_eq!(verified, Rect::new(121.0, 120.0, 420.0, 312.0));
        // Just beyond tolerance -> mismatch + rollback.
        let mut ops = MockOps::new(Rect::new(100.0, 668.0, 420.0, 312.0));
        ops.read_override = Some(Rect::new(121.5, 120.0, 420.0, 312.0));
        let err = set_window_frame_txn(&ops, EL, target).unwrap_err();
        assert!(matches!(err, AxError::VerificationFailed { .. }));
    }

    // MARK: - Multi-window observation (pure builder; no Apple frameworks)

    #[test]
    fn observation_assigns_one_key_per_window_and_promotes_focused() {
        let observation = observation_from_windows(
            Rect::new(0.0, 0.0, 1200.0, 800.0),
            &[
                (
                    WindowKey(0),
                    Rect::new(10.0, 10.0, 200.0, 200.0),
                    WindowRole::Application,
                ),
                (
                    WindowKey(1),
                    Rect::new(220.0, 10.0, 200.0, 200.0),
                    WindowRole::Application,
                ),
            ],
            Some(WindowKey(1)),
        );
        assert_eq!(observation.snapshot.windows.len(), 2);
        assert_eq!(observation.snapshot.displays.len(), 1);
        assert_eq!(
            observation.snapshot.displays[0].work_area,
            observation.snapshot.displays[0].bounds
        );
        assert_eq!(observation.map.get(WindowId(0)), Some(WindowKey(0)));
        assert_eq!(observation.map.get(WindowId(1)), Some(WindowKey(1)));
        assert_eq!(observation.map.len(), 2);
        assert_eq!(observation.snapshot.focused_window, Some(WindowId(1)));
    }

    #[test]
    fn observation_ignores_focused_window_that_was_not_observed() {
        let observation = observation_from_windows(
            Rect::new(0.0, 0.0, 1200.0, 800.0),
            &[(
                WindowKey(0),
                Rect::new(0.0, 0.0, 100.0, 100.0),
                WindowRole::Application,
            )],
            Some(WindowKey(7)),
        );
        assert_eq!(observation.snapshot.focused_window, None);
    }
}
