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

### Spike 1 addendum — batch multi-window application (macOS, recorded result)

Implemented with best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)), explicitly **not atomic**. Recorded on the owner's ThinkPad X13 Yoga Gen 1, macOS Tahoe 26.7.1 build 25G241, AX granted; batch code and CLI from branch `work/phase1-domain` commit `085f5cb`; runs on 2026-10-22 (single-fixture) and 2026-10-23 (two-window + denied path). Targets are self-owned `HYPR-SPIKE-TARGET` windows only; no titles read.

| Case | Command | Result |
|---|---|---|
| CLI gates (no AX call when they fail) | `ax-probe batch` variants | no args → usage, exit 1; no/bad fixture registration → exit 5; plan must include the registered fixture and be a regular non-symlink owner-only (0600) file |
| Two-window success | `batch --target <A> --token <hex> --plan [A, B]` | both frames applied and verified; `rollback_all_restored=true`, exit 0; per-window `frame` read-backs matched |
| Two-window rollback | plan `[A→new frame, B→new frame, <live pid with no window>]` | third target failed (`-25204 cannotComplete`) after A and B were applied; batch stopped and restored A and B, both verified by read-back; exit 1 |
| Permission-denied | read-only `ax-probe frame 1` in an AX-untrusted launchd user-domain job | `error: permission denied (Accessibility not granted)`, exit 3; no AX call made; one context only (see `ARENA-VERIFICATION.md` §6) |

**Exact host/build:** as in Spike 0 record; AX granted to the controlling terminal app. **Evidence & caveats:** `docs/ARENA-VERIFICATION.md` §6 — executed on the owner's host, not independently run by arena.ai; second window B came from the committed `spawn-target --no-register` mode of the spike harness (auxiliary target only; spike harness merged into `main` via PR #1 — evidence run used spike head `7636eab`); batch code and CLI built from `work/phase1-domain` `085f5cb` (merged via PR #5).

### Spike 1 addendum — per-window runtime identity and planner wiring (macOS, recorded result)

The batch above addressed targets by pid, which cannot name more than one window of a process. This addendum closes that gap (host run recorded 2026-10-23 on the owner's machine):

- `MacAxAdapter::windows(pid)` enumerates `AXWindows` for one pid, checks each element's owning pid (`AXUIElementGetPid`), reads its frame, and records the focused window (`kAXFocusedWindowAttribute` + `CFEqual`). The result is a `MacWindowSet` that owns the element handles and assigns one opaque runtime `WindowKey` per window. `BatchOps` is implemented for `MacWindowSet` (never for a pid), so a key is never re-resolved against a window list that changed after enumeration.
- `MacWindowSet::observe(display)` returns a `PlatformSnapshot` plus a `WindowIdMap` (planner `WindowId` → native `WindowKey`). `batch::targets_for_plan` maps a `LayoutPlan` through that map and fails closed on an unmapped ID, so the domain never sees a native key.
- CLI: `ax-probe windows <pid>` (read-only key listing), `ax-probe apply --target <pid> --token <hex> --display <x y w h> [--kind <name>]` (snapshot → planner → mapping → batch), and `ax-probe batch --plan` whose lines are now `key x y width height` (the rows above predate the format change). Every plan key must exist in the fixture's enumerated window set.
- Limits: `display` is supplied by the caller (no monitor query; `bounds == work_area`, scale `1.0`); capability bits are optimistic because the M3 review established that `AXUIElementIsAttributeSettable` is not a reliable eligibility gate — the verified write plus rollback is the real gate; subrole (dialog) classification is not implemented.

**Host run (2026-10-23, owner host ThinkPad X13 Yoga Gen 1, Tahoe 26.7.1, AX granted to the controlling terminal):** the fixture `spawn-target --count 2` (built from `spikes/macos/spike-macos.swift`) opened two self-owned `HYPR-SPIKE-TARGET` windows in **one** process. `ax-probe windows <pid>` listed both with distinct runtime keys; `ax-probe apply --display 0 24 1920 1056` computed a master-stack plan (master = the focused window), applied both targets, `rollback_all_restored=true`, exit 0, verified by read-back; a second run was idempotent. `ax-probe batch --plan` applied the same frames; a plan naming an unobserved key failed closed (`UnknownTarget`) with no writes.

**Coordinate convention (recorded):** AX frames use a **top-left origin**, so the `--display` work area must exclude the menu bar. Passing `0 0 1920 1080` placed a window under the menu bar; macOS clamped it and the read-back mismatched (y 12→30), so the batch rolled back and reported `rollback_all_restored=true` with the restored frame — the rollback path working as documented. The adapter does not query the work area yet; the caller supplies it.

### Spike 1 addendum — minimal macOS menu-bar shell (recorded result)

`ax-shell` is a Rust `NSStatusItem` shell (AppKit through `unsafe` FFI confined to `ffi.rs`/`shell.rs`, no new dependencies): a status line, one `Reflow: <kind>` command, `Pause`, a settings summary, `Reset to defaults`, an Accessibility status line, `Diagnostics` and `Quit`. Auto-tiling stays **off** — nothing is applied until the user picks a reflow. The work area is the main display bounds (`CGDisplayBounds`) minus a documented 30 pt menu-bar inset, matching the AX top-left origin.

**Config file (small settings slice):** the shell is driven by an owner-only `~/.hypr-rust/config.json` — a flat JSON object documented in `crates/platform-macos/src/config.rs`: `version`, `layout` (`master-stack`/`columns`/…), `outer_gap`, `inner_gap`, `master_ratio`, `auto_tiling` (default `false`), `include_dialogs`. The file must be a regular non-symlink file owned by the current user with perm `0600`, checked both when loading and after every write (temp file + rename, no dependencies added). Known keys with a wrong value type and out-of-range numbers (gaps `< 0`, `master_ratio` outside 0.05–0.95, unknown `layout`/`version`) are rejected; unknown keys are ignored for forward compatibility. A missing file means defaults; any other load error is logged and defaults are used.

**Host run (2026-10-24, owner host, AX granted, self-owned fixture):** `ax-shell --self-test` built the menu and read its 10 items back through AppKit, then ran one reflow; the two fixture windows moved from `(100,668,420,312)` / `(620,348,420,312)` to `(12,42,1038,1026)` / `(1058,42,850,1026)`, verified by `ax-probe windows`. `ax-shell` (the run loop) stayed alive, so the status item was created.

**Host run 2 — config-driven reflow (2026-10-24, owner host, AX granted, same fixture):** with `{"layout":"columns","outer_gap":20,"inner_gap":4,"master_ratio":0.6,…}` (perm `0600`) the menu read back `Reflow: columns` and `settings: layout=columns gaps=20/4 ratio=0.6 auto-tiling=off dialogs=out`, and the reflow placed the two windows at `(20,50,938,1010)` / `(962,50,938,1010)` — the exact arithmetic of the work area `(0,30,1920,1050)` minus the configured gaps. Replacing the config with a world-readable `0644` file made the load fail closed with `config: config must be a regular owner-only 0600 file` and fall back to defaults (`Reflow: master-stack`, gaps 12/8); `ax-shell --reset-defaults` then wrote the default config back as `0600` and the reflow reproduced `(12,42,1038,1026)` / `(1058,42,850,1026)`. Rust checks on `platform-macos`: fmt OK, clippy `-D warnings` clean, lib tests 33/33 (+2 `ax-keyboard`); config parser/writer covered by 6 unit tests incl. an atomic-write round-trip and the `0600` guard.

**Not done:** no event loop or auto-tiling, no settings UI (the config file backs the shell), no hotkeys, no diagnostics UI (`Diagnostics` appends to `~/.hypr-rust/shell.log`), no signed bundle. Note: `-[NSMenuItem initWithTitle:action:keyEquivalent:]` raised an Objective-C exception on this build, so items are created with `+new` plus `setTitle:`/`setAction:`/`setTarget:` instead.

### Spike 1 addendum — per-window subrole classification (recorded result)

Devices/windows carries a role the planner's exclusion logic consumes: `OwnApplication` (pid == controller pid — mandatory, not overridable), `System` (notification surfaces), `Dialog` (dialog/floating subroles — excluded unless allowed by the plan's dialog policy), ordinary `Application` otherwise. On Tahoe 26.7.1 a floating utility `NSPanel` reports `AXSystemFloatingWindow` and a plain `NSWindow` reports `AXStandardWindow` (runtime-observed through a Swift probe script; `AXDialogSubrole`/`AXSystemDialogSubrole`/`AXFloatingWindowSubrole` are the canonical dialog subroles kept in the policy list). `ax-probe windows` prints `key=<n> role=<application|dialog|own-application|system|desktop>` (still no titles). The `spawn-target` harness grew `--dialog` (one floating utility panel next to the ordinary windows).

**Host run (2026-10-24, owner host, AX granted, self-owned fixture `--count 2 --dialog`):**
- Enumeration classified the panel `role=dialog` and both ordinary windows `role=application`.
- Default reflow (dialogs excluded by policy): `master-stack: applied 2 window(s), 1 excluded`; the dialog frame stayed exactly `(1350,465,320,224)` before and after read-back; the two application windows tiled as in the earlier records.
- `include_dialogs=true` flipped on: the plan covered all three windows, but the **floating panel rejected the size write** (AX `-25200` after the position write) — the batch stopped at that key and the verified rollback restored the panel to its exact pre-write frame `(1350,465,320,224)`; the other two windows kept their already-correct tiling frames. This closes the loop on the M3 rule that the *write-time* verified-write-plus-rollback is the real gate, while `AXUIElementIsAttributeSettable` (which reports `settable=false` for `AXSize` on the panel class) is not.
- Restored `include_dialogs=false` re-run was idempotent (`applied 2 window(s), 1 excluded`).
- Rust checks: fmt OK, clippy `-D warnings` clean; platform-macos lib **35/35** (+2 `ax-keyboard`) with classification/wiring unit tests; workspace **23/23**.

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
- Machine model and CPU: **REKONSILIASI 2026-10-18** — label lama "Intel MacBook Pro 13" class" (imprecise) diganti dengan bukti sistem: `system_profiler` melaporkan Model Identifier **MacBookPro16,2** (MacBook Pro 14" 2019) sementara owner melaporkan perangkat fisik **Lenovo ThinkPad X13 Yoga Gen 1**; CPU i5-10310U konsisten dengan ThinkPad. Kemungkinan model identifier dilaporkan firmware/EFI (SMBIOS), bukan label perangkat fisik. Kedua nilai dicatat; tidak ada serial/UUID/EFI config dibaca. Lihat `docs/reports/MACOS-M1-AX-ADAPTER-2026-10-18.md` §1.
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
