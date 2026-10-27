# ADR-0007: Global Hotkey for Reflow (⌥⌘R) via the Carbon Event Manager

- **Status:** Accepted (owner decision 2026-10-27, choosing roadmap item "global hotkeys" alongside auto-tiling as the next slice); **amended 2026-10-28** — chord ⌃⌥⌘R → ⌥⌘R (owner decision, host evidence, see §1 amendment)
- **Date:** 2026-10-27
- **Decision owners:** Product + owner
- **Relates to:** [ADR-0006](./0006-real-window-reflow-strict-no-partial.md) (the reflow command this hotkey triggers), [ADR-0001](./0001-product-boundary.md), docs/M2-KEYBOARD-PLAN.md (whose "no global hotkey" rule was **scope-bound to M2** and is superseded here for the shell surface)

## Context

The menu is the owner-driven non-fixture surface of ADR-0006; every reflow so far
(host-validated 2026-10-27: refusal legs + `tiled N window(s)` success) was started by a
menu click. The delivery roadmap lists **global hotkeys with conflict reporting** as an
outstanding Phase 2 requirement ("platform-default hotkeys and conflict reporting"), and
the owner chose it as the next slice.

M2's decisions recorded "no global hotkey daemon" — that bound **M2's acceptance** (a
fixture-only, one-shot CLI slice), not the product. This ADR is the new decision that
opens the feature.

## Decision

### 1. One hotkey, one action

- **⌥⌘R** (option + command + `R`; Carbon key code `kVK_ANSI_R = 0x0F`,
  modifiers `optionKey|cmdKey = 2048|256 = 2304`) triggers **exactly the same
  action as the menu's current-layout reflow**: `perform_reflow_real`. Same preflight,
  same strict no-partial contract, same three-way status line, same log format, same
  paused gate. There is no second code path and no separate semantics.
- The chord was chosen deliberately: `RegisterEventHotKey` intercepts the chord
  **system-wide before the frontmost app sees it**, so an everyday chord (e.g. ⌘⇧R,
  browser reload) would be hijacked while the shell runs. ⌥⌘R is unlikely to collide
  with any app's own shortcut (and any collision stays fail-soft per §3).
- One press = one reflow attempt (one-shot). **No loop, no daemon, no auto-tiling** —
  ADR-0006's "nothing is applied unless the user picks a reflow command" still holds;
  a hotkey press *is* an explicit user command.

**Amendment (2026-10-28, owner): chord ⌃⌥⌘R → ⌥⌘R.** The chord accepted on
2026-10-27 (⌃⌥⌘R, `controlKey|optionKey|cmdKey = 4096|2048|256`) **never once
matched on the owner's machine** — zero press lines in the shell log across every
attempt. Read-only host probes (timestamped `CGEventSourceFlagsState` sessions,
2026-10-27/28) found the root cause **outside ax-shell**: whenever ⌃ is pressed
**together with another modifier**, the HID system reports the `shift` bit instead
of `ctrl` (`alt+shift` and `alt+shift+cmd` recorded during ⌃⌥ / ⌃⌥⌘ instructions),
so any chord containing ⌃ plus another modifier can never match on this keyboard
(internal ThinkPad PS/2 unit, VoodooPS2Controller 2.3.8; no software remap —
`hidutil` `UserKeyMapping` empty, no modifier-mapping plist). The replacement was
proven end-to-end on this machine **before** the code changed: a throwaway C
listener on the same Carbon API registered ⌥⌘R (`RegisterEventHotKey` returned
`0`, chord free of conflicts) and logged `FIRED … (⌥⌘R)` 14 times with HID flags
`alt=1 cmd=1` on every press — HID → match → dispatch all alive. ⌥⌘R also avoids
claiming a lone Option chord (⌥R alone would swallow `®` in editors) and does not
collide with browser reload (⌘R). ⌥R / ⌃R remain one-constant swaps should the
owner prefer a different chord later. **Everything else in this ADR is unchanged.**

### 2. Public API only: Carbon Event Manager

- Registration uses **`RegisterEventHotKey` / `UnregisterEventHotKey` +
  `InstallEventHandler`** from the Carbon Event Manager (`Carbon.framework`), linked
  directly with no new crate dependency. These are documented, supported 64-bit public
  APIs (widely used by menu-bar apps); they do **not** require the Input Monitoring /
  Accessibility prompt that a `CGEventTap` would, and they add **no new TCC permission**
  beyond the Accessibility trust the shell already holds.
- Carbon event handlers are delivered on the process's main run loop, which is the
  `NSApplication` run loop the shell already uses; the handler runs on the main thread,
  exactly like a menu action.

### 3. Fail-soft on conflict, never fatal

- If registration fails — the chord is already registered by another app
  (`eventHotKeyExistsErr = -9878`) or any other `OSStatus` — the shell **logs the status
  code, keeps running, and stays fully usable through the menu**. A hotkey conflict is an
  inconvenience, not a startup failure.
- Registration happens once in `run()` (the menu shell). The fixture-bound automated
  paths (`--self-test`, `--panel-click-proof`) **never register a hotkey**, keeping
  automated evidence exactly as it is.
- The hotkey is unregistered on quit (the OS would release it at process exit anyway;
  explicit unregister keeps the teardown honest).

### 4. Surface class: non-fixture, owner-driven

- The hotkey joins the menu as a **non-fixture surface** (ADR-0006 §3): it may target
  real app windows and requires the owner's hand (pressing the chord). Automated evidence
  remains fixture-bound; host verification = the owner presses the chord and the log
  records the same status line a menu click produces.

## Consequences

- The roadmap's "global hotkeys" item is satisfied for the reflow command; configurable
  chords, per-layout hotkeys and a conflict-resolution UI are **out of scope** for now
  (a fixed platform-default chord with conflict reporting is the roadmap requirement).
- Pressing ⌥⌘R anywhere consumes that chord system-wide while ax-shell runs — accepted,
  and why the chord is an unusual combination.
- If a future macOS build deprecates `RegisterEventHotKey`, the fallback decision (e.g.
  `CGEventTap` + its Input Monitoring prompt, or per-app `NSEvent` monitors) needs its
  own decision; nothing here claims those.
- Evidence expectations: unit tests cover the pure parts (chord constants, status-code
  reporting); live registration is host evidence (owner press), not fixture evidence.
