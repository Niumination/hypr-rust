//! Pure decision logic of event-driven auto-tiling —
//! [ADR-0008](../../docs/adr/0008-auto-tiling-event-driven.md).
//!
//! This module holds the unit-testable parts only: the **coalescer** (a
//! burst of window open/close notifications collapses into at most one
//! re-tile per tick) and the **trigger filter** (an app with an `exclude`
//! rule triggers nothing — ADR-0008 decision 4). The AXObserver wiring
//! that feeds these — attach/detach on the main run loop, `NSWorkspace`
//! app notifications, the timer, the paused gate — lives in the shell and
//! lands in a later PR; until then nothing in the crate calls this module,
//! which is exactly that PR's "no behavior change" contract. The automatic
//! action is and stays `perform_reflow_real` itself (ADR-0008 §3).

use std::collections::HashMap;
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::{DialogPolicy, WindowRule};
use crate::ffi;
use crate::shell;

/// Collapses a burst of window open/close events into one scheduled action.
///
/// ADR-0008 §3: notifications arrive in bursts (launch sequences, dialogs
/// spawning windows). Any number of [`on_event`](Self::on_event) calls
/// before the coalescing timer fires yield **at most one** `true` from
/// [`take_tick`](Self::take_tick) — one re-tile per burst, latest state
/// wins, no queue. The timer itself (a single pending ~500 ms main-run-loop
/// timer) is wiring, not logic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coalescer {
    pending: bool,
}

impl Coalescer {
    /// A quiet coalescer: no event since the last tick.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a window open/close event. Pure state — starting or extending
    /// the coalescing timer is the wiring's job.
    pub fn on_event(&mut self) {
        self.pending = true;
    }

    /// The coalescing timer fired. Returns `true` when at least one event
    /// arrived since the previous fire — the caller then runs exactly one
    /// re-tile. A second fire without a new event returns `false`.
    pub fn take_tick(&mut self) -> bool {
        std::mem::replace(&mut self.pending, false)
    }
}

/// ADR-0008 decision 4: whether the app identified by `pid`/`bundle` may
/// trigger an automatic re-tile. An app matched by an `exclude` rule (the
/// ADR-0006 app-protection policy, `dialogs: Exclude`) triggers nothing —
/// it behaves like a floating window. Rules with `Include`/`Default`
/// policy never block triggering, and a rule that does not match the
/// source (e.g. a bundle rule against an app without a bundle id) leaves
/// triggering allowed.
pub fn app_may_trigger(rules: &[WindowRule], pid: u32, bundle: Option<&str>) -> bool {
    !rules
        .iter()
        .any(|rule| rule.dialogs == DialogPolicy::Exclude && rule.matches(pid, bundle))
}

// MARK: - Wiring (ADR-0008 §2–§4) — attach/detach, callback, coalescing timer
//
// Everything here runs on the main thread/run loop (§4): attach and detach
// are called from the shell's enable path, `run()` startup, the quit
// teardown and the NSWorkspace app notifications; the AX callback and the
// timer fire on the main run loop. `unsafe` is confined to raw CF/AX calls
// whose refs are owned exactly as documented per call.

/// Coalescing window: one re-tile per burst, latest state wins (§3).
const COALESCE_SECONDS: f64 = 0.5;

/// Whether the shell currently wants automatic reflow (the panel toggle).
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Automated-evidence mode (`--self-test` / `--panel-click-proof`): these
/// paths never attach observers, whatever the config says (§5).
static AUTOMATED: AtomicBool = AtomicBool::new(false);

static COALESCER: Mutex<Coalescer> = Mutex::new(Coalescer { pending: false });
/// The one pending one-shot coalescing timer (created lazily, released on
/// fire or on disable).
static TICK_TIMER: Mutex<Option<TickTimer>> = Mutex::new(None);
/// Live observers: pid → observer + the elements its notifications are
/// registered on (app element + one owned retain per observed window).
static OBSERVED: Mutex<Option<HashMap<u32, ObservedApp>>> = Mutex::new(None);

/// The pending coalescing timer ref, held in a static (see [`ObservedApp`]
/// for the same stance: main-thread use only, `Send` lets the static hold
/// it and licenses no cross-thread access).
struct TickTimer(ffi::CFRunLoopTimerRef);

// SAFETY: same justification as `ObservedApp` — created, fired, invalidated
// and released exclusively on the main thread (schedule_tick / cancel_tick
// / timer_fired, all main run loop).
unsafe impl Send for TickTimer {}

/// One app's observer set. The refs are owned: the observer and the app
/// element come from their create/copy calls, each window element from a
/// `CFRetain` (the AXWindows array only borrows); `detach_app` releases
/// every one of them exactly once.
struct ObservedApp {
    observer: ffi::AXObserverRef,
    app_element: ffi::AXUIElementRef,
    windows: Vec<ffi::AXUIElementRef>,
}

// SAFETY: every ref here is touched only on the main thread — attach and
// detach from the shell's enable/startup/quit/workspace paths, the AX
// callback and the timer, all delivered on the main run loop. `Send` only
// lets the static *hold* them; it does not license cross-thread use (the
// same stance as `hotkey::Registered`).
unsafe impl Send for ObservedApp {}

/// Process-lifetime CFStrings built once and intentionally leaked (the
/// adapter's `Key` pattern): the two notification names and the run-loop
/// mode are constants for the whole session.
fn notification_created() -> ffi::CFStringRef {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE
        .get_or_init(|| ffi::cf_string_from_static(ffi::K_AX_WINDOW_CREATED_NOTIFICATION) as usize)
        as ffi::CFStringRef
}

fn notification_destroyed() -> ffi::CFStringRef {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        ffi::cf_string_from_static(ffi::K_AX_UI_ELEMENT_DESTROYED_NOTIFICATION) as usize
    }) as ffi::CFStringRef
}

fn mode_cfstring() -> ffi::CFStringRef {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| ffi::cf_string_from_static(ffi::K_CF_RUN_LOOP_DEFAULT_MODE) as usize)
        as ffi::CFStringRef
}

fn ax_windows_attr() -> ffi::CFStringRef {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| ffi::cf_string_from_static(b"AXWindows\0") as usize) as ffi::CFStringRef
}

/// Mark the automated-evidence paths so `set_enabled` can never attach an
/// observer there (ADR-0008 §5: their evidence stays exactly as it is).
pub fn set_automated(automated: bool) {
    AUTOMATED.store(automated, Ordering::SeqCst);
}

/// Apply the shell's auto-tiling toggle (ADR-0008 §1: live, no relaunch).
///
/// Enabling re-syncs the observer set with the current rules and apps;
/// disabling removes every observer and cancels a pending tick. Called
/// from the panel Save path, `run()` startup and the quit teardown. In
/// automated-evidence mode this is a no-op.
pub fn set_enabled(enabled: bool) {
    if AUTOMATED.load(Ordering::SeqCst) {
        return;
    }
    let was = ENABLED.swap(enabled, Ordering::SeqCst);
    if enabled {
        detach_all();
        let attached = attach_all();
        if was != enabled || attached > 0 {
            shell::append_log(&format!(
                "auto-tiling: enabled — observers on {attached} app(s)"
            ));
        }
    } else {
        cancel_tick();
        let detached = detach_all();
        if was || detached > 0 {
            shell::append_log(&format!(
                "auto-tiling: disabled — observers removed ({detached} app(s))"
            ));
        }
    }
}

/// Attach observers to every eligible regular app (startup / enable).
/// Returns how many apps were attached; failures are logged once per app
/// and degrade triggers, never writes (ADR-0008 §4).
fn attach_all() -> usize {
    let own = unsafe { ffi::getpid() } as u32;
    let mut attached = 0;
    for pid in shell::regular_app_pids() {
        if pid == own {
            continue;
        }
        let bundle = shell::bundle_id_for_pid(pid);
        if !app_may_trigger(&shell::current_rules(), pid, bundle.as_deref()) {
            continue; // decision 4: excluded apps are never observed
        }
        if attach_app(pid) {
            attached += 1;
        }
    }
    attached
}

/// Remove every observer and release all refs. Returns the count removed.
fn detach_all() -> usize {
    let taken: Vec<u32> = {
        let observed = OBSERVED.lock().unwrap();
        observed
            .as_ref()
            .map(|map| map.keys().copied().collect())
            .unwrap_or_default()
    };
    for pid in &taken {
        detach_app(*pid);
    }
    taken.len()
}

/// Create the observer for one app: `AXWindowCreated` on the app element
/// plus `AXUIElementDestroyed` on each of its current windows.
fn attach_app(pid: u32) -> bool {
    {
        let observed = OBSERVED.lock().unwrap();
        if observed.as_ref().is_some_and(|map| map.contains_key(&pid)) {
            return true;
        }
    }
    // SAFETY: `callback` (module-level, outlives any observer); `out` is a
    // valid out-pointer receiving an owned ref on success (AXUIElement.h).
    let (status, observer) = unsafe {
        let mut observer: ffi::AXObserverRef = ptr::null_mut();
        let status = ffi::AXObserverCreate(pid as c_int, on_ax_notification, &mut observer);
        (status, observer)
    };
    if status != 0 || observer.is_null() {
        shell::append_log(&format!(
            "auto-tiling: observer unavailable for pid {pid} (AXError {status})"
        ));
        return false;
    }
    // SAFETY: `observer` owned by this function; `app_element` is a fresh
    // owned ref or null (both create calls documented in ffi.rs).
    let app_element = unsafe { ffi::AXUIElementCreateApplication(pid as c_int) };
    if app_element.is_null() {
        // SAFETY: owned ref from AXObserverCreate, released exactly once here.
        unsafe {
            ffi::CFRunLoopRemoveSource(
                ffi::CFRunLoopGetMain(),
                ffi::AXObserverGetRunLoopSource(observer),
                mode_cfstring(),
            );
            ffi::CFRelease(observer as ffi::CFTypeRef);
        }
        shell::append_log(&format!(
            "auto-tiling: app element unavailable for pid {pid}"
        ));
        return false;
    }
    let mut windows = Vec::new();
    // SAFETY: all refs owned/live; the mode CFString is the leaked constant.
    unsafe {
        ffi::CFRunLoopAddSource(
            ffi::CFRunLoopGetMain(),
            ffi::AXObserverGetRunLoopSource(observer),
            mode_cfstring(),
        );
        let status = ffi::AXObserverAddNotification(
            observer,
            app_element,
            notification_created(),
            ptr::null_mut(),
        );
        if status != 0 {
            ffi::CFRunLoopRemoveSource(
                ffi::CFRunLoopGetMain(),
                ffi::AXObserverGetRunLoopSource(observer),
                mode_cfstring(),
            );
            ffi::CFRelease(app_element as ffi::CFTypeRef);
            ffi::CFRelease(observer as ffi::CFTypeRef);
            shell::append_log(&format!(
                "auto-tiling: window-create observation refused for pid {pid} (AXError {status})"
            ));
            return false;
        }
        // Observe window close on every window the app has right now. The
        // AXWindows array only borrows its elements, so each one is
        // retained (released in detach_app).
        let mut list: ffi::CFTypeRef = ptr::null();
        if ffi::AXUIElementCopyAttributeValue(app_element, ax_windows_attr(), &mut list) == 0
            && !list.is_null()
        {
            let count = ffi::CFArrayGetCount(list);
            for index in 0..count {
                let raw = ffi::CFArrayGetValueAtIndex(list, index) as ffi::AXUIElementRef;
                if raw.is_null() {
                    continue;
                }
                let owned = ffi::CFRetain(raw) as ffi::AXUIElementRef;
                if ffi::AXObserverAddNotification(
                    observer,
                    owned,
                    notification_destroyed(),
                    ptr::null_mut(),
                ) == 0
                {
                    windows.push(owned);
                } else {
                    ffi::CFRelease(owned as ffi::CFTypeRef);
                }
            }
            ffi::CFRelease(list);
        }
    }
    let mut observed = OBSERVED.lock().unwrap();
    observed.get_or_insert_with(HashMap::new).insert(
        pid,
        ObservedApp {
            observer,
            app_element,
            windows,
        },
    );
    true
}

/// Remove one app's notifications, run-loop source and every owned ref.
fn detach_app(pid: u32) {
    let app = {
        let mut observed = OBSERVED.lock().unwrap();
        observed.as_mut().and_then(|map| map.remove(&pid))
    };
    let Some(app) = app else { return };
    // SAFETY: every ref is owned by this module and released exactly once;
    // the remove calls pair with the adds in attach_app (AXUIElement.h).
    unsafe {
        for window in app.windows {
            ffi::AXObserverRemoveNotification(app.observer, window, notification_destroyed());
            ffi::CFRelease(window as ffi::CFTypeRef);
        }
        ffi::AXObserverRemoveNotification(app.observer, app.app_element, notification_created());
        ffi::CFRelease(app.app_element as ffi::CFTypeRef);
        ffi::CFRunLoopRemoveSource(
            ffi::CFRunLoopGetMain(),
            ffi::AXObserverGetRunLoopSource(app.observer),
            mode_cfstring(),
        );
        ffi::CFRelease(app.observer as ffi::CFTypeRef);
    }
}

/// AX delivery: one call per observed notification, on the main run loop.
///
/// Window created → track the new window's close notification, then
/// coalesce; window destroyed → drop our retained copy, then coalesce.
/// The trigger filter is re-checked here (rules can change at runtime).
extern "C" fn on_ax_notification(
    _observer: ffi::AXObserverRef,
    element: ffi::AXUIElementRef,
    notification: ffi::CFStringRef,
    _refcon: *mut c_void,
) {
    let mut pid_c: c_int = 0;
    // SAFETY: `element` is a live element delivered by the observer.
    if unsafe { ffi::AXUIElementGetPid(element, &mut pid_c) } != 0 {
        return;
    }
    let pid = pid_c as u32;
    // SAFETY: `notification` is the live name the observer delivered;
    // the constants are the leaked CFStrings we registered with.
    let created = unsafe {
        ffi::CFEqual(
            notification as ffi::CFTypeRef,
            notification_created() as ffi::CFTypeRef,
        ) != 0
    };
    // Trace point 1 of the automatic path (diagnostic): proves the AX
    // callback itself was delivered. One line per observed window event.
    shell::append_log(&format!(
        "auto-tiling: event {} from pid {pid}",
        if created {
            "AXWindowCreated"
        } else {
            "AXUIElementDestroyed"
        }
    ));
    let bundle = shell::bundle_id_for_pid(pid);
    if !app_may_trigger(&shell::current_rules(), pid, bundle.as_deref()) {
        return; // decision 4, re-checked: rules may have changed since attach
    }
    {
        let mut observed = OBSERVED.lock().unwrap();
        let Some(map) = observed.as_mut() else { return };
        let Some(app) = map.get_mut(&pid) else { return };
        if created {
            // The created notification delivers the new window element
            // (borrowed) — retain it and start observing its close.
            // SAFETY: `element` is live; the retain pairs with detach_app.
            let owned = unsafe { ffi::CFRetain(element) as ffi::AXUIElementRef };
            // SAFETY: owned element + owned observer, matching add in attach_app.
            let added = unsafe {
                ffi::AXObserverAddNotification(
                    app.observer,
                    owned,
                    notification_destroyed(),
                    ptr::null_mut(),
                )
            };
            if added == 0 {
                app.windows.push(owned);
            } else {
                // SAFETY: the add failed, so this retain is ours to release.
                unsafe { ffi::CFRelease(owned as ffi::CFTypeRef) };
            }
        } else if let Some(position) = app.windows.iter().position(|window| {
            // SAFETY: both refs are live CF objects (CFEqual).
            unsafe { ffi::CFEqual(*window as ffi::CFTypeRef, element as ffi::CFTypeRef) != 0 }
        }) {
            let window = app.windows.remove(position);
            // SAFETY: owned refs; the remove pairs with the add in attach_app
            // (it may report the dying element — the error is expected here).
            unsafe {
                ffi::AXObserverRemoveNotification(app.observer, window, notification_destroyed());
                ffi::CFRelease(window as ffi::CFTypeRef);
            }
        }
    }
    // One burst → one action (ADR-0008 §3).
    COALESCER.lock().unwrap().on_event();
    schedule_tick();
}

/// Ensure the single pending one-shot coalescing timer exists.
fn schedule_tick() {
    let mut slot = TICK_TIMER.lock().unwrap();
    if slot.is_some() {
        return; // already coalescing — the pending tick will see the event
    }
    // SAFETY: the callout is this module's function, alive for the session;
    // the timer is created one-shot and released on fire or cancel.
    unsafe {
        let timer = ffi::CFRunLoopTimerCreate(
            ptr::null(),
            ffi::CFAbsoluteTimeGetCurrent() + COALESCE_SECONDS,
            0.0,
            0,
            0,
            timer_fired,
            ptr::null_mut(),
        );
        if timer.is_null() {
            return;
        }
        ffi::CFRunLoopAddTimer(ffi::CFRunLoopGetMain(), timer, mode_cfstring());
        *slot = Some(TickTimer(timer));
        // Trace point 1b (diagnostic): the callback reached scheduling.
        shell::append_log("auto-tiling: coalescing timer scheduled");
    }
}

/// Release the fired one-shot timer's ref, leaving the coalescer intact.
/// The fired timer is already invalid (one-shot); invalidate before the
/// release so nothing can race it (all on the main thread anyway).
fn release_fired_timer() {
    if let Some(TickTimer(timer)) = TICK_TIMER.lock().unwrap().take() {
        // SAFETY: the timer ref is owned by the slot and released exactly
        // once here; a fired one-shot timer is safe to invalidate.
        unsafe {
            ffi::CFRunLoopTimerInvalidate(timer);
            ffi::CFRelease(timer as ffi::CFTypeRef);
        }
    }
}

/// Cancel a pending tick without acting (disable path): drops the timer
/// **and** the pending event, so a later enable starts from a quiet state.
fn cancel_tick() {
    release_fired_timer();
    COALESCER.lock().unwrap().take_tick();
}

/// Timer fire (main run loop): one re-tile per burst, dropped silently
/// while paused (ADR-0008 §3) and while disabled.
///
/// The fired timer's ref is released **without** touching the coalescer —
/// the pending event is exactly what this fire exists to read (fix
/// 2026-10-30: calling `cancel_tick` here cleared the pending flag before
/// `take_tick` could read it, so every automatic reflow silently never
/// ran; host trace showed events arriving with zero ticks).
extern "C" fn timer_fired(_timer: ffi::CFRunLoopTimerRef, _info: *mut c_void) {
    release_fired_timer();
    if !ENABLED.load(Ordering::SeqCst) {
        // Disabled while the tick was pending: drop the stale event.
        COALESCER.lock().unwrap().take_tick();
        return;
    }
    if COALESCER.lock().unwrap().take_tick() {
        // Trace point 2 of the automatic path (diagnostic): proves the
        // coalescing timer fired with at least one pending event.
        shell::append_log("auto-tiling: coalesced tick — automatic reflow");
        shell::perform_reflow_automatic();
    }
}

/// A newly launched regular app joins the observation (no-op when
/// disabled; the own-app and exclude checks mirror attach_all).
pub fn on_app_launched(pid: u32) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let own = unsafe { ffi::getpid() } as u32;
    if pid == own {
        return;
    }
    let bundle = shell::bundle_id_for_pid(pid);
    if !app_may_trigger(&shell::current_rules(), pid, bundle.as_deref()) {
        return;
    }
    attach_app(pid);
}

/// A terminated app's observer set is released (map empty when disabled).
pub fn on_app_terminated(pid: u32) {
    detach_app(pid);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exclude(bundle: &str) -> WindowRule {
        WindowRule {
            bundle: Some(bundle.to_string()),
            pid: None,
            dialogs: DialogPolicy::Exclude,
        }
    }

    #[test]
    fn disabling_without_observers_is_a_safe_noop_and_automated_mode_never_enables() {
        // Teardown of an empty observer set touches no FFI: safe off-host.
        set_enabled(false);
        // Automated evidence paths (ADR-0008 §5) must never attach, whatever
        // the config says — set_enabled is a hard no-op there.
        set_automated(true);
        set_enabled(true);
        assert!(!ENABLED.load(std::sync::atomic::Ordering::SeqCst));
        set_automated(false);
        set_enabled(false);
    }

    #[test]
    fn coalescer_fires_once_per_burst_and_stays_quiet_until_the_next_event() {
        let mut coalescer = Coalescer::new();
        // Nothing happened: the first tick is quiet.
        assert!(!coalescer.take_tick());
        // A burst of three events is one action, and only one.
        coalescer.on_event();
        coalescer.on_event();
        coalescer.on_event();
        assert!(coalescer.take_tick());
        assert!(!coalescer.take_tick());
        // A later, separate event starts a new burst.
        coalescer.on_event();
        assert!(coalescer.take_tick());
        assert!(!coalescer.take_tick());
    }

    #[test]
    fn coalescer_defaults_to_quiet() {
        assert!(!Coalescer::default().take_tick());
    }

    #[test]
    fn excluded_app_triggers_nothing_and_ordinary_apps_trigger() {
        let rules = vec![exclude("com.mobyrix.Nicegram")];
        assert!(!app_may_trigger(&rules, 477, Some("com.mobyrix.Nicegram")));
        assert!(app_may_trigger(&rules, 42, Some("com.apple.finder")));
        // No rules at all: every regular app may trigger.
        assert!(app_may_trigger(&[], 42, Some("com.apple.finder")));
    }

    #[test]
    fn include_and_default_rules_never_block_triggering() {
        let rules = vec![
            WindowRule {
                bundle: Some("com.a".to_string()),
                pid: None,
                dialogs: DialogPolicy::Include,
            },
            WindowRule {
                bundle: Some("com.b".to_string()),
                pid: None,
                dialogs: DialogPolicy::Default,
            },
        ];
        assert!(app_may_trigger(&rules, 1, Some("com.a")));
        assert!(app_may_trigger(&rules, 1, Some("com.b")));
    }

    #[test]
    fn pid_exclude_rule_matches_by_pid_only() {
        let rules = vec![WindowRule {
            bundle: None,
            pid: Some(900),
            dialogs: DialogPolicy::Exclude,
        }];
        assert!(!app_may_trigger(&rules, 900, None));
        assert!(!app_may_trigger(&rules, 900, Some("com.anything")));
        assert!(app_may_trigger(&rules, 901, None));
    }

    #[test]
    fn bundle_rule_never_matches_an_app_without_a_bundle_id() {
        let rules = vec![exclude("com.a")];
        // Unsigned CLI apps legitimately report no bundle id; the rule
        // matcher stays fail-closed on the bundle side (config.rs).
        assert!(app_may_trigger(&rules, 7, None));
    }
}
