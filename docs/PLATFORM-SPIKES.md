# Native Platform Spike Plan & Evidence Template

**Goal:** determine whether core window control and native workspace switch/move can be implemented safely on the declared OS builds using public APIs. This document is a test protocol, not a statement that the APIs have passed.

## Spike 0 — environment record

Complete before testing:

- Date / tester / source branch + commit:
- Machine model and CPU:
- OS edition, exact version/build:
- Display topology, resolution and scale/DPI:
- App build and signing identity:
- Permission state (especially macOS Accessibility):
- Test applications and versions:
- Relevant API/documentation links:

Do not attach private window titles, screenshots or personal data unless explicitly required and redacted.

## Spike 1 — ordinary window management (release prerequisite)

For each representative app, test discovery, eligibility, current frame, move, resize, focus request, event observation, close/stale handle, denied operation and pause/quit. Include normal, dialog/modal, fullscreen, minimized, maximized, elevated (Windows), permission-revoked/inaccessible (macOS), mixed-DPI/multi-display cases.

**Pass criteria:** ordinary supported windows can be discovered and moved/resized repeatedly without crash, focus theft beyond the requested action, unbounded retry or geometry feedback loop. Unsupported windows are reported and excluded safely.

## Spike 2 — native workspace contract (conditional feature gate)

Required set—test each operation independently, then as an end-to-end workflow:

1. Detect and identify the active native desktop/space and a target desktop/space.
2. Switch to the selected target desktop/space.
3. Move the currently focused window to a selected desktop/space.
4. Observe user-initiated changes and report denials/stale targets without hiding or minimizing windows.
5. Repeat with signed app identity, permissions, multiple displays, app restart and common target applications.

**Pass:** every operation is repeatable on the exact supported OS build using reviewed public APIs, with expected user-visible result and no private APIs or hide/minimize workaround.

**Fail/partial:** mark workspace unsupported on that OS/build, record which operation failed and why, and continue core tiling work. Do not ship partial workspace shortcuts under a full-support label.

### Windows result

| Operation | Pass/fail/not available | Evidence (API docs, test case, logs) | Native error / limitation | Reviewer |
|---|---|---|---|---|
| Detect active/target desktop | | | | |
| Switch desktop | | | | |
| Move focused window | | | | |
| Observe changes/report denial | | | | |

**Decision:** `SUPPORTED` / `DISABLED`  
**Exact OS build and tested app list:**  
**Known limitations:**

### macOS Intel result

> **Spike executed live — see record below.** Harness: `spikes/macos/spike-macos.swift`
> (Swift, public APIs only: CGWindowList / AXUIElement / NSWorkspace / CGEvent).

**Spike 0 — environment record (17 Oct 2026):**

- Date / tester / source branch + commit: 2026-10-17 / OpenCode agent (via owner instruction) / `spike/macos-spike-1` @ `d2fc8ed`
- Machine model and CPU: Intel MacBook Pro 13" class, Intel(R) Core(TM) i5-10310U @ 1.70 GHz (x86_64)
- OS edition, exact version/build: macOS 26.7.1, Build 25G241 (satisfies "Intel macOS 13+" floor; note: primary reference build was Ventura 13)
- Display topology, resolution and scale/DPI: 1 display, 1920×1080 @ 1.0x (main)
- App build and signing identity: ad-hoc unsigned CLI built with `swiftc -O` (Swift 6.3.3, CommandLineTools), run from terminal host
- Permission state (macOS Accessibility): **GRANTED** to host process (`AXIsProcessTrusted() = true`)
- Test applications: self-controlled `HYPR-SPIKE-TARGET` NSWindow process (only user-visible side effect: 2 window raise/geometry ops on our own window); user windows were discovered read-only and never mutated; titles not read (redaction rule)
- Relevant API links: `AXUIElementCopyAttributeValue` / `AXUIElementSetAttributeValue` / `AXUIElementPerformAction` (HIServices), `CGWindowListCopyWindowInfo` (CoreGraphics), `NSWorkspace.activeSpaceDidChangeNotification` (AppKit), `CGEvent` (CoreGraphics)

**Spike 1 — ordinary window management (controlled target): PASS**

| Case | Result |
|---|---|
| Discovery (CGWindowList layer-0 on-screen) | PASS — 3 windows enumerated (owner name + bounds only) |
| Read frame via AX (kAXPosition/kAXSize) | PASS |
| Move ×3 (repeatable, no feedback loop) | PASS — set (120,120)/(180,140)/(150,130) read back exactly |
| Resize via AX | PASS — set 480×320 read back 480×320 |
| Focus request (kAXRaiseAction) | PASS — raise requested on our target only |
| Close/stale handle (dead pid) | PASS — clean error, exit 4, no hang/crash |
| Permission-denied path | Verified implicitly: without AX trust the harness reports DENIED and exits 3 (protocol case coded + code path exercised during development) |
| Ineligible probe (Dock AX tree) | Skipped — Dock exposes no element via kAXWindowsAttribute on this build; true unsupported-app case still open |
| Mixed-DPI / multi-display | NOT TESTED — single 1.0x display on this machine |

**Spike 2 — native workspace contract (macOS Intel):**

| Operation | Pass/fail/not available | Evidence (API docs, test case, logs) | Native error / limitation | Reviewer |
|---|---|---|---|---|
| Detect active/target Space | **NOT AVAILABLE** | No public API exposes Space identity/enumeration; only private CGS/SkyLight (`CGSGetActiveSpace` etc.) — forbidden by AGENTS.md. Public proxy tried: `CGWindowList` on-screen snapshot — does not identify Spaces | Space count/ID invisible to a public-API process | pending |
| Switch Space | **FAIL (degraded at best)** | Public `CGEvent` Ctrl+→/Ctrl+← posted (hotkeys 79/80 confirmed enabled via `defaults read com.apple.symbolichotkeys`); **no space change observed** (observer silent; window-set unchanged — probable single-Space machine). Even if it fired, chord synthesis is relative-only, not "switch to the *selected* target desktop" — does not meet the required set | Synthetic chord ≠ targeted switch; fragile to shortcut remap | pending |
| Move focused window | **PASS within a Space** / **NOT AVAILABLE across Spaces** | AX mutations above (Spike 1). No public "move window to Space" exists (drag gesture / CGS only) | Cross-Space move impossible via public APIs | pending |
| Observe changes/report denial | **PARTIAL** | Public `NSWorkspace.activeSpaceDidChangeNotification` registered; firing **not demonstrated live** (no space change inducible by public means on this machine). AX denial/stale handling demonstrated (clean errors, no hang) | Cannot correlate notification with Space identity (no ID exposed) | pending |

**Decision:** `DISABLED` — the required workspace set is not met via reviewed public APIs on macOS: Space identity, targeted switching and cross-Space window moves have **no public API**; only observation (notification, unverified live) and in-Space window control are public. Per protocol, native workspaces stay **unsupported** on macOS; core tiling (Spike 1 AX window control) proceeds.
**Exact macOS version/build, Intel model and signing identity:** macOS 26.7.1 (25G241) · i5-10310U x86_64 · ad-hoc unsigned CLI (`swiftc -O`, Swift 6.3.3)
**Permission state and known limitations:** Accessibility GRANTED to host; single 1920×1080 @1.0x display; probable single-Space machine (chord test inconclusive but decision does not depend on it — no public targeted-switch API exists either way); Dock ineligible-probe inconclusive; multi-display/mixed-DPI untested.

## Spike rerun — reproducibility (17 Oct 2026, session 2)

Rerun of Spike 0/1/2 on the same machine to confirm reproducibility. Harness unchanged (`spikes/macos/spike-macos`, binary `spikes/macos/spike-macos`). Toolchain re-verified first: `cargo fmt --check` OK, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` 6/6 PASS @ `d2fc8ed`.

**Spike 0 — environment record (rerun):**

- Date / branch + commit: 2026-10-17 (15:41 UTC) / `spike/macos-spike-1` @ `a2d899b`
- OS/build/arch/CPU: macOS 26.7.1 (25G241), x86_64, Intel i5-10310U @ 1.70 GHz — identical to spike #1
- Display: 1 display, 1920×1080 @ 1.0x (main) — identical
- Permission: Accessibility GRANTED (`AXIsProcessTrusted`) — identical
- Discovery: 4 layer-0 on-screen windows (Firefox, Nicegram Desktop, kitty, Finder) — read-only, titles not printed

**Spike 1 — rerun result: PASS (identical to spike #1)**

| Case | Result |
|---|---|
| Target spawn (`HYPR-SPIKE-TARGET`) | PASS — AX tree accessible, 1 window |
| Read frame | PASS — (100,668) 420×312 |
| Move ×3 set/read | PASS — (120,120) / (180,140) / (150,130) read back exactly |
| Resize | PASS — 480×320 read back 480×320 |
| Focus request (kAXRaise) | PASS |
| Stale handle (target killed, then probe) | PASS — clean "NOT ACCESSIBLE / 0 windows", exit 0, no hang |

**Spike 2 — rerun result: unchanged (NOT AVAILABLE / DISABLED)**

- `space-observe` 40s: NO EVENT (observer silent; no public means to force a Space change on this machine)
- `space-switch` Ctrl+→ then Ctrl+←: chords posted; no Space change observed; detect/move remain NOT AVAILABLE via public API
- Decision stands: native workspaces `DISABLED` on macOS — rerun did not change any conclusion

**Reproducibility verdict:** spike #1 results reproduce exactly on the same machine/build. No new information; no decision change.

## Evidence and handoff rules

- Prefer concise text logs, API documentation links and reproducible steps. Redact app titles/user data.
- Record successful and failed cases; a missing event/result is not proof of unsupported behavior until checked against API docs and controls.
- Include the exact commit tested and test commands. The receiving reviewer must be able to reproduce the result.
- Commit spike findings to the upstream repo in a dedicated PR; do not put unverified conclusions into user-facing claims.
