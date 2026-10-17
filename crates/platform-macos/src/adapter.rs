//! Safe macOS Accessibility adapter. All `unsafe` FFI lives in [`super::ffi`];
//! this module only composes safe wrappers with documented invariants.
//!
//! Scope (M1): one self-owned target window per pid. No enumeration of user
//! windows, no titles, no system-UI mutation, no workspaces, no private APIs.

use std::fmt;
use std::ptr;
use std::sync::OnceLock;

use layout_core::Rect;

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

/// Typed failure modes. Every AXError code maps to exactly one variant so the
/// executor can branch on the real cause instead of a generic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The size write was rejected after the position write had already
    /// been applied. The original position was restored (or the restore
    /// failed — `rollback_restored` says which), so the caller knows the
    /// window is (or is not) left in a partial state.
    SizeAfterPositionFailed {
        source: Box<AxError>,
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
            Self::SizeAfterPositionFailed {
                source,
                rollback_restored: true,
            } => write!(
                f,
                "size write rejected after position applied ({source}); original position restored"
            ),
            Self::SizeAfterPositionFailed {
                source,
                rollback_restored: false,
            } => write!(
                f,
                "size write rejected after position applied ({source}); ROLLBACK FAILED - window left at new position with old size"
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

/// Validate frame geometry before any write: finite coordinates and
/// positive, finite size.
fn validate_frame_rect(rect: Rect) -> Result<(), AxError> {
    if rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
    {
        Ok(())
    } else {
        Err(AxError::InvalidGeometry)
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
    /// 1. Preflight: geometry must be finite with positive size
    ///    ([`AxError::InvalidGeometry`] otherwise — nothing is written).
    /// 2. The current geometry is saved before the first write.
    /// 3. Position is written, then size. If the size write is rejected,
    ///    the saved position is restored and verified by read-back; the
    ///    error ([`AxError::SizeAfterPositionFailed`]) states explicitly
    ///    whether the rollback succeeded, so a rejected operation never
    ///    leaves the window changed without a clear report.
    pub fn set_window_frame(&self, window: &AxWindow, rect: Rect) -> Result<(), AxError> {
        validate_frame_rect(rect)?;
        // Save first: if the size write fails after the position write
        // lands, this is the rollback target.
        let before = self.window_frame(window)?;
        self.write_point(window.element(), k_ax_position_attribute(), rect.x, rect.y)?;
        if let Err(source) = self.write_size(
            window.element(),
            k_ax_size_attribute(),
            rect.width,
            rect.height,
        ) {
            // Rollback: restore the saved position, then verify by reading
            // the frame back. Restore failure is reported, not hidden.
            let restore = self.write_point(
                window.element(),
                k_ax_position_attribute(),
                before.x,
                before.y,
            );
            let rollback_restored = restore.is_ok()
                && self
                    .window_frame(window)
                    .map(|r| (r.x - before.x).abs() <= 1.0 && (r.y - before.y).abs() <= 1.0)
                    .unwrap_or(false);
            return Err(AxError::SizeAfterPositionFailed {
                source: Box::new(source),
                rollback_restored,
            });
        }
        Ok(())
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
        };
        assert!(rolled_back
            .to_string()
            .contains("original position restored"));
        let not_restored = AxError::SizeAfterPositionFailed {
            source: Box::new(AxError::Unsupported),
            rollback_restored: false,
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
}
