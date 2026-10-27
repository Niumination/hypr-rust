//! Global hotkey for reflow — [ADR-0007](../../docs/adr/0007-global-hotkey-reflow.md).
//!
//! One chord (**⌃⌥⌘R**) registered once at shell startup through the Carbon
//! Event Manager's public `RegisterEventHotKey` API (no new TCC permission,
//! no crate dependency). A press dispatches the exact callback the caller
//! passes — the shell passes `perform_reflow_real`, so a hotkey press and a
//! menu click are indistinguishable in semantics, status line and log.
//!
//! Conflict handling is fail-soft (ADR-0007 §3): if the chord is already
//! taken (`eventHotKeyExistsErr = -9878`) registration returns a human-readable
//! error; the shell logs it and keeps running menu-only.
//!
//! All `unsafe` and raw Carbon types for this feature live in this module;
//! [`crate::ffi`] only declares them.

use std::os::raw::c_void;
use std::ptr;
use std::sync::Mutex;

use crate::ffi;

/// Application signature for our `EventHotKeyID` (`'hypr'`, big-endian) so
/// the handler can recognize its own hotkeys among any others in-process.
const SIGNATURE: u32 = u32::from_be_bytes(*b"hypr");
/// The id of our single registered hotkey within this process.
const REFLOW_HOTKEY_ID: u32 = 1;

/// Live Carbon refs while registered; dropped by [`unregister`].
struct Registered {
    hotkey: ffi::EventHotKeyRef,
    handler: ffi::EventHandlerRef,
}

/// SAFETY justification: the two Carbon refs are opaque pointers touched
/// only while holding the `REGISTERED` mutex, and every touch happens on
/// the main thread — registration in `run()`, the press handler (Carbon
/// delivers on the main run loop) and teardown in quit. `Send` here only
/// lets the static *hold* them; it does not license cross-thread use.
unsafe impl Send for Registered {}

/// Both fields are only touched on the main thread (registration in `run()`,
/// the Carbon handler, and teardown in quit) — the mutex just makes the
/// statics legal without claiming a broader threading model.
static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);
static ON_PRESS: Mutex<Option<fn()>> = Mutex::new(None);

/// The reflow chord: `⌃⌥⌘R` as `(key_code, modifiers)` —
/// `kVK_ANSI_R` plus `controlKey | optionKey | cmdKey`
/// (4096 | 2048 | 256 = 6400). Pure; unit-tested.
pub fn reflow_chord() -> (u32, u32) {
    (
        ffi::K_VK_ANSI_R,
        ffi::K_MOD_CONTROL | ffi::K_MOD_OPTION | ffi::K_MOD_COMMAND,
    )
}

/// Human-readable report for a registration `OSStatus`. The conflict case
/// (`eventHotKeyExistsErr`) is named explicitly — this is the roadmap's
/// "conflict reporting" (ADR-0007 §3). Pure; unit-tested.
pub fn describe_status(status: ffi::OSStatus) -> String {
    if status == ffi::OSSTATUS_EVENT_HOT_KEY_EXISTS {
        "chord already in use by another app (eventHotKeyExistsErr)".to_string()
    } else {
        format!("OSStatus {status}")
    }
}

/// Register the chord and install the press handler. Idempotent: a second
/// call while registered succeeds without touching the existing refs.
///
/// `on_press` runs synchronously on the main thread — the same thread and
/// semantics as a menu action — so ordering with UI updates is unchanged.
/// Failure at either step unwinds the other (no half-registration).
pub fn register(on_press: fn()) -> Result<(), String> {
    let mut registered = REGISTERED.lock().unwrap();
    if registered.is_some() {
        return Ok(());
    }
    // Callback first: events can only arrive after the hotkey exists, and
    // the handler reads this slot when they do.
    *ON_PRESS.lock().unwrap() = Some(on_press);

    let (key_code, modifiers) = reflow_chord();
    // SAFETY: `GetApplicationEventTarget` returns the process-lifetime
    // target; `id` is passed by value; `hotkey_out` is a valid out-pointer
    // that receives an owned ref on success.
    let (status, hotkey) = unsafe {
        let target = ffi::GetApplicationEventTarget();
        let id = ffi::EventHotKeyID {
            signature: SIGNATURE,
            id: REFLOW_HOTKEY_ID,
        };
        let mut hotkey: ffi::EventHotKeyRef = ptr::null_mut();
        let status = ffi::RegisterEventHotKey(key_code, modifiers, id, target, 0, &mut hotkey);
        (status, hotkey)
    };
    if status != ffi::OSSTATUS_NO_ERR {
        *ON_PRESS.lock().unwrap() = None;
        return Err(describe_status(status));
    }

    let spec = ffi::EventTypeSpec {
        class: ffi::K_EVENT_CLASS_KEYBOARD,
        kind: ffi::K_EVENT_HOT_KEY_PRESSED,
    };
    // SAFETY: `handler` outlives registration (removed in `unregister`);
    // the spec is passed by pointer only for the duration of the call;
    // `handler_out` receives an owned ref on success.
    let (status, handler) = unsafe {
        let target = ffi::GetApplicationEventTarget();
        let mut handler: ffi::EventHandlerRef = ptr::null_mut();
        let status = ffi::InstallEventHandler(
            target,
            on_hotkey_event,
            1,
            &spec,
            ptr::null_mut(),
            &mut handler,
        );
        (status, handler)
    };
    if status != ffi::OSSTATUS_NO_ERR {
        // SAFETY: `hotkey` is the owned ref this function just received.
        unsafe { ffi::UnregisterEventHotKey(hotkey) };
        *ON_PRESS.lock().unwrap() = None;
        return Err(describe_status(status));
    }

    *registered = Some(Registered { hotkey, handler });
    Ok(())
}

/// Release both Carbon refs. Returns an error message when either release
/// fails so the caller can log it; the state slot is cleared regardless
/// (the refs are never used again).
pub fn unregister() -> Option<String> {
    let taken = REGISTERED.lock().unwrap().take()?;
    let mut errors = Vec::new();
    // SAFETY: both refs are owned by this module and were returned by the
    // matching install/register calls; they are used once, here.
    unsafe {
        let status = ffi::UnregisterEventHotKey(taken.hotkey);
        if status != ffi::OSSTATUS_NO_ERR {
            errors.push(format!("unregister hotkey: {}", describe_status(status)));
        }
        let status = ffi::RemoveEventHandler(taken.handler);
        if status != ffi::OSSTATUS_NO_ERR {
            errors.push(format!("remove handler: {}", describe_status(status)));
        }
    }
    *ON_PRESS.lock().unwrap() = None;
    if errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    }
}

/// Carbon press handler. Reads the `EventHotKeyID` parameter and, when it is
/// ours, dispatches the registered callback on the main thread. Returns
/// `noErr` (handled) or `eventNotHandledErr` per CarbonEventsCore.h: any
/// other code would stop event propagation.
extern "C" fn on_hotkey_event(
    _call: ffi::EventHandlerCallRef,
    event: ffi::EventRef,
    _user_data: *mut c_void,
) -> ffi::OSStatus {
    let mut id = ffi::EventHotKeyID {
        signature: 0,
        id: 0,
    };
    // SAFETY: `event` is a live EventRef delivered by the Carbon Event
    // Manager to this handler; `id` is a valid buffer of
    // `size_of::<EventHotKeyID>()`, which is what `buffer_size` declares;
    // the out-parameters may be null and are passed as such.
    let status = unsafe {
        ffi::GetEventParameter(
            event,
            ffi::K_EVENT_PARAM_DIRECT_OBJECT,
            ffi::K_TYPE_EVENT_HOT_KEY_ID,
            ptr::null_mut(),
            std::mem::size_of::<ffi::EventHotKeyID>() as u64,
            ptr::null_mut(),
            &mut id as *mut ffi::EventHotKeyID as *mut c_void,
        )
    };
    if status != ffi::OSSTATUS_NO_ERR || id.signature != SIGNATURE || id.id != REFLOW_HOTKEY_ID {
        return ffi::OSSTATUS_EVENT_NOT_HANDLED;
    }
    match *ON_PRESS.lock().unwrap() {
        Some(on_press) => {
            on_press();
            ffi::OSSTATUS_NO_ERR
        }
        // Registered chord with no callback — handled but nothing to do.
        None => ffi::OSSTATUS_NO_ERR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflow_chord_is_control_option_command_r() {
        let (code, modifiers) = reflow_chord();
        // kVK_ANSI_R = 0x0F (Events.h).
        assert_eq!(code, 0x0F);
        // controlKey 4096 | optionKey 2048 | cmdKey 256 (Events.h bits).
        assert_eq!(modifiers, 4096 | 2048 | 256);
        assert_eq!(modifiers, 6400);
        // No shift: the chord must not depend on Caps-Lock-adjacent bits.
        assert_eq!(modifiers & 512, 0);
    }

    #[test]
    fn conflict_status_is_reported_by_name_and_others_by_code() {
        let text = describe_status(-9878);
        assert!(text.contains("already in use"), "{text}");
        assert!(text.contains("eventHotKeyExistsErr"), "{text}");
        let other = describe_status(-9870);
        assert_eq!(other, "OSStatus -9870");
        // noErr never reaches describe_status, but the format stays sane.
        assert_eq!(describe_status(0), "OSStatus 0");
    }

    #[test]
    fn signature_is_the_four_char_code_hypr() {
        // 'hypr' big-endian: h=0x68, y=0x79, p=0x70, r=0x72.
        assert_eq!(SIGNATURE, 0x6879_7072);
        assert_eq!(REFLOW_HOTKEY_ID, 1);
    }

    #[test]
    fn register_is_idempotent_and_unregister_is_safe_without_registration() {
        // No Carbon calls in tests unless a registration already exists:
        // unregister with nothing registered is a no-op.
        assert!(unregister().is_none());
        // Without a live NSApplication event loop the Carbon registration in
        // a test process is not asserted here — live registration is host
        // evidence (ADR-0007 consequences), pressed by the owner.
    }
}
