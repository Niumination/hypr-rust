# ADR-0009 (Draft): Hypr-Rust Extension Protocol and Waybar-Like Integration

- **Status:** Proposed — owner selected the transport and capability direction on 2026-11-04; wire schema and implementation details remain for review
- **Date:** 2026-11-04
- **Decision owners:** Product + owner
- **Relates to:** ADR-0001 (product boundary), ADR-0006 (strict real-window reflow), ADR-0007 (global hotkey), ADR-0008 (event-driven auto-tiling)

## Context

The owner confirmed a long-term goal for Hypr-Rust to provide a Mac-native Rust tiling-window-manager experience inspired by Hyprland, including a Waybar-like status surface and an extensible plugin ecosystem.

The current upstream is a native macOS window controller using Accessibility/AppKit. It is not a Wayland compositor and does not replace WindowServer. That boundary affects compatibility:

- Waybar is designed as a status bar for Wayland compositors (including Sway/wlroots); its compositor-facing surface/modules cannot be used unchanged as a native macOS bar.
- Hyprland plugins use Hyprland's own plugin API and are built for its compositor process/API/ABI. They are not binary-compatible plugins for a macOS app.
- The achievable goal is **behavioral/configuration interoperability** and a Hypr-Rust extension API, not loading existing Waybar or Hyprland binaries unchanged.

## Problem

A bar or extension must be able to observe Hypr-Rust state and, eventually, request an explicitly allowed action without reaching into the window-control implementation, bypassing ADR-0006, exposing private window data, or adding an unsafe in-process ABI.

## Options considered

1. **Load original Hyprland plugins in-process.** Rejected: compositor ABI/internal assumptions do not match this app; this would not create a Hyprland compositor on macOS and would expand crash/security impact.
2. **Run Waybar unchanged as the macOS bar.** Rejected as the default path: Waybar's Wayland compositor integration has no corresponding native macOS layer-shell surface in the current product architecture. Reuse of selected module concepts or custom-script output may be explored later through an adapter.
3. **Create a versioned Hypr-Rust local protocol and native Mac bar/extensions.** Proposed: isolate the protocol from AX/AppKit details, start read-only, and add narrowly permissioned actions only after the safety model is accepted.

## Proposed decision

### Owner-selected direction (2026-11-04)

- Start with a **versioned local IPC** (Unix-domain socket), not a bar-only in-process prototype.
- External extensions are **read-only by default**. The owner wants them eventually to be able to request selected actions, but only through separately granted, explicit capabilities. No extension may bypass the app's validated action path or directly call AX.
- Initial implementation should establish a read-only status/event contract. Add a narrowly scoped action capability only in a later, separately tested step.

### 1. Compatibility contract

- “Waybar/Hyprland-like compatibility” means compatible user concepts, selected config semantics, status/event meanings, and adapters where feasible.
- It does **not** promise binary compatibility with Waybar modules or Hyprland C++ plugins.
- No claim of compositor-level functionality, native Spaces control, or replacement of WindowServer is introduced by this ADR.

### 2. Protocol boundary

- Use an owner-local, versioned IPC endpoint; proposed transport is a Unix-domain socket within the private `~/.hypr-rust/` state directory. Do not expose a network listener.
- The first implementation is deliberately narrower than the long-term protocol: UTF-8 JSON Lines (one newline-terminated request per connection), bounded to 4096 bytes, with a restricted ASCII request ID, explicit version, and structured errors. No JSON dependency was added; the parser rejects escapes, unknown/duplicate fields, and non-empty params rather than partially accepting them.
- Version 1 currently implements only **read-only `status.get`**. Its snapshot contains the active layout name, paused state, configured auto-tiling state, dialog-inclusion setting, and app version. No event subscription, window list, reflow counts, or mutation method is implemented yet.
- Wire example for the implemented subset:
  ```json
  {"version":1,"id":"req-1","method":"status.get","params":{}}
  {"version":1,"id":"req-1","ok":true,"result":{"layout":"columns","paused":false,"auto_tiling":false,"include_dialogs":false,"app_version":"0.1.0"}}
  ```
  `scripts/ipc-status.py` is a dependency-free sample client for this snapshot. Any future streaming/subscription method needs separate reconnect, backpressure, and event-loss semantics before implementation.
- Never publish window titles, document names, full AX trees, or arbitrary application text. Expose only the minimum status needed by a bar.
- A future write-capable protocol may request named shell actions (e.g. select a layout or request a reflow), but must carry an explicitly granted capability and route through the same validated shell action and ADR-0006 semantics. Plugins may not directly call AX or supply arbitrary window geometry.

### 3. Extension process and permissions

- Start with external extension processes communicating over IPC; do not load third-party `.dylib` code into the app process.
- The prototype tightens `~/.hypr-rust/` to owner-only, creates an owner-only socket (`0600`), validates the socket owner/type, and checks the connected peer's effective UID with macOS `getpeereid(2)`; any lookup failure or different UID is denied.
- Read-only access is the default. Any mutation capability is separately declared, visible to the user, opt-in, revocable, and still constrained by mandatory exclusions and strict reflow reporting.
- Define extension identity/version, protocol version, declared capabilities, lifecycle, timeout, and failure behavior before publishing an SDK. A crashed/unresponsive extension must not block reflow or the main menu.

### 4. Bar integration

- First consumer is a native macOS status surface using the same protocol/data model (initially the existing `NSStatusItem`; a richer native panel is a later product decision).
- Candidate modules: active layout, auto-tiling on/off, pause state, last reflow result, fitting/skipped window counts, and supported/unsupported capability indicators.
- A “workspaces” module must say unavailable while macOS Spaces remain disabled; it must not fabricate workspace IDs.
- Future adapters may translate a documented subset of Waybar-like module/config concepts. They must identify unsupported modules rather than silently pretend to support them.

## Staged rollout (proposal)

1. **Protocol ADR + schema review:** settle event names, privacy, permissions, config relationship, and dependency choice.
2. **Read-only IPC prototype (shipped via PR #49, amended 2026-11-07):** `status.get` snapshot over the bounded owner-only Unix socket; the pure parser/response tests run in CI and on the host (platform-macos 103 lib + 2 bin, workspace 29). Native validation executed on the Tahoe host on 2026-11-07 (ARENA Path R): version mismatch, malformed/oversize input, same-UID peer behavior, stale socket, client disconnect, quit cleanup, a live query from `scripts/ipc-status.py`, and proof that the path cannot mutate windows (mutation method rejected) all pass. **Still not runtime-tested:** the other-UID peer gate (single-user host) and the second-instance bind-conflict path. Event streaming is explicitly deferred.
3. **Native bar consumer (first consumer = the existing `NSStatusItem`, owner decision 2026-11-07):** render the same read-only status through AppKit and verify lifecycle/permission behavior on the Tahoe host; a richer screen-edge panel is a later product decision.
4. **External extension SDK:** only after protocol stability; publish sample read-only extension and lifecycle tooling.
5. **Action requests, decided 2026-11-07:** only `layout.select` and `reflow.request`, persisted per extension, revocable and visible in settings, with first-run confirmation — implemented as a separate, later, tested step that never weakens ADR-0006 or claims original plugin ABI compatibility. Interop order is native module API → Waybar-style custom-script adapter → config translator.

## Acceptance and security tests (before any write-capable extension)

- IPC remains owner-local, owner-only, bounded, and unavailable over TCP/network.
- Unknown protocol versions, malformed messages, unauthorized peers, oversized payloads, and extension timeouts fail closed.
- Read-only v1 emits no titles, tokens, credentials, or AX element data and cannot move windows.
- Extension crashes/restarts do not crash the shell or trigger reflow.
- If action requests are later enabled, all window mutations pass through the existing strict real-window path; fixture-only automated evidence remains fixture-only.
- No new dependency is introduced without review; CI covers protocol unit/integration tests and the macOS native transport.

## Owner decisions (2026-11-07 — the three review questions are resolved)

1. **First bar consumer = the existing `NSStatusItem`** rendered from the same read-only protocol. A richer second screen-edge panel stays a later product decision; no new UI surface is introduced in the first consumer step.
2. **Waybar-like interop order = native module API first, custom-script adapter second, config translator last.** The native Hypr-Rust module API consumes the contract our own bar uses; a Waybar-style `custom/…` module adapter (documented JSON text output) follows once the protocol is stable; translating full Waybar config semantics comes only after that, and unsupported modules must be named rather than faked.
3. **Eligible action capabilities = `layout.select` and `reflow.request` only**, both routed through the same validated shell actions (`reflow.request` = the identical strict ADR-0006 path as the menu and ⌥⌘R). A grant **persists per extension**, is revocable and visible in the settings UI, with a **first-run confirmation** when the extension first requests it. Anything beyond these two named actions, and any direct AX access for extensions, stays out of scope.

## References

- Waybar upstream describes itself as a customizable bar for Wayland compositors: <https://github.com/Alexays/Waybar/blob/master/README.md>
- Hyprland plugin development/API: <https://wiki.hypr.land/Plugins/Development/Getting-Started/>
- Apple `getpeereid(3)` manual page for same-user Unix-domain peer credentials: <https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/getpeereid.3.html>
- Hypr-Rust roadmap: `docs/ROADMAP.md` (ecosystem design after the Mac core stabilizes)
