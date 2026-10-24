# Arena.ai Verification Brief — hypr-rust

**Purpose:** let an external verifier (arena.ai) check every public claim in this repo without access to the owner's host. Everything below is either (a) reproducible from the public repo alone, or (b) explicitly marked as host-bound evidence with the reason.

**Repo:** https://github.com/Niumination/hypr-rust (public) · local: `labs/hypr-rust/`  
**Date:** 2026-10-19 (updated after arena.ai round-5 review — C9 softened, C14 lock TOCTOU, honest limits). **Updated 2026-10-22** — added owner-reported Path A + read-only multi-window probe (§6), host-bound table, and [ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md) (batch accepted with best-effort rollback, superseding [ADR-0004](./adr/0004-batch-multiwindow-deferred-no-partial.md)); tests are owner-reported, not independently run by arena.ai.

---

## 1. Artifact map (all public)

`Commit` = latest **code** commit for that row's content. A docs-only tip may
sit above it.

| Branch | Commit | PR | Content |
|---|---|---|---|
| `main` | `efaaba4` | — | Code bundle: M1 AX adapter + M2 keyboard-first merged. Later docs-only commits add the Hackintosh host profile, this brief, and ADR-0004 — the row points to the code commit; docs tips are not pinned |
| `spike/macos-spike-1` | `0eb9a25` | [#1](https://github.com/Niumination/hypr-rust/pull/1) | Swift AX spike (Dock mutation removed), Spaces DISABLED, host-label reconciliation, lifecycle-bounded fixture registration (token + flock, no unused helpers, token not printed) |
| `core/tiling-constraints` | `d814d2c` | [#2](https://github.com/Niumination/hypr-rust/pull/2) | Min-size constraints + overflow policy (13 tests) |
| `macos/tahoe-ax-adapter-m1` | `e78d104` | [#3](https://github.com/Niumination/hypr-rust/pull/3) | M1 Rust AX adapter (rollback on all post-mutation failures + flock-gated registration, EWOULDBLOCK-only lock check); merged to main |
| `macos/m2-keyboard-plan` | `27191d7` | [#4](https://github.com/Niumination/hypr-rust/pull/4) | M2 keyboard-first plan + `ax-keyboard` implementation (select/move/resize/raise); all-variants error formatter + dispatch; merged to main |

Platform-macos is deliberately **outside** workspace members: `cargo build/test --workspace` stays Windows-safe; the macOS crate is built via its own manifest (see §4).

## 2. Claims → evidence → how to verify

| # | Claim | Evidence (public) | Verify command |
|---|---|---|---|
| C1 | layout-core compiles clean, 6 tests pass | `crates/layout-core` | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` |
| C2 | Constrained planner: min-size clamp + diagnostics, order preserved, deterministic | PR #2 (merged into `main`) + 13 tests incl. LCG property test | `cargo test -p hypr-rust-layout-core` on merged `main` (13 tests) |
| C3 | Swift spike: AX read/move/resize/raise PASS on self-owned window; Spaces NOT AVAILABLE via public API; Dock mutation removed | `docs/PLATFORM-SPIKES.md`, `spikes/macos/spike-macos.swift` | `grep -ri dock spikes/` → comments only; `grep -n "dock" spikes/macos/spike-macos.swift` → removal note; workspace section documents NOT AVAILABLE |
| C4 | Rust adapter discovers/moves/resizes/raises a self-owned window via public AX/CoreGraphics FFI | `docs/reports/MACOS-M1-AX-ADAPTER-2026-10-18.md` §5 (verbatim run) | requires macOS host with AX grant — see §3 |
| C5 | Stale target → typed error, exit 4, prompt, no crash/retry/false-success | report §5; `adapter.rs` (`kill(pid,0)` + errno ESRCH check, `AxError::StaleTarget`) | `kill <pid>; ax-probe frame <pid>` → exit 4 |
| C6 | No private APIs, no deps beyond workspace path-dep, no secrets | `crates/platform-macos/Cargo.toml` (single path dep); `ffi.rs` (only ApplicationServices/CoreFoundation/libSystem public symbols) | `grep -rE "SkyLight|CGSPrivate|SLSC" crates/` → empty |
| C7 | Core is platform-neutral (no Apple frameworks in shared core) | `crates/layout-core` has zero deps, no `cfg(target_os)` | `cargo tree -p hypr-rust-layout-core` → empty |
| C8 | Host identity: Hackintosh (OpenCore SMBIOS spoof `MacBookPro16,2`) on ThinkPad X13 Yoga Gen 1 | `docs/USER-DEVICE-PROFILE.md` §"Host identity — confirmed Hackintosh" (evidence table + EFI `SystemProductName` read) | owner-host bound — see §3 |
| C9 | Preflight + best-effort rollback; recovery failure reported (NOT atomic) | report §5/§6/§7; `adapter.rs` `validate_frame_rect` (`Rect::is_finite_and_positive`), `set_window_frame_txn` (PositionWriteFailed / SizeAfterPositionFailed / VerificationReadFailed / VerificationFailed) | `ax-probe set-frame --target <pid> --token <hex> nan 0 100 100` → exit 1, window unchanged; 9 mock unit tests; AX not transactional → rollback best-effort (§7) |
| C10 | Workspace builds without macOS | root `Cargo.toml` members = layout-core only; platform-macos standalone `[workspace]` | `cargo test --workspace` on any OS → layout-core only, no Apple link |
| C11 | (F1) Mutation commands require a token matching the self-owned fixture registration; otherwise refused before any AX call | `fixture.rs` `verify_registration` (file regular non-symlink 0600 uid-match, pid+token match, pid alive, flock lock held — only EWOULDBLOCK/EAGAIN accepted as 'held', other errors reject), exit 5; `adapter.rs` `process_alive` | `ax-probe set-frame <pid> 1 2 3 4` → exit 5; live run §5 |
| C12 | (F2) `set_window_frame` read-back verification: mismatch → full rollback + typed `VerificationFailed`; all post-mutation failures roll back | `adapter.rs` `frames_match` + `set_window_frame_txn` (PositionWriteFailed / SizeAfterPositionFailed / VerificationReadFailed / VerificationFailed); 9 mock unit tests | unit tests 33/33; live run §5 (all read-backs matched) |
| C13 | (F1b) Fixture registration is lifecycle-bounded; rejects stale records when the lock is free (lock does NOT bind fixture identity to the AX operation — see C14) | `spike-macos.swift` `acquireFixtureLock`/`writeRegistration`/`removeRegistration` (flock held for fixture lifetime, atomic 0600 write, cleanup on terminate); `ax-probe.rs` `verify_fixture_lock` | live run §5: lock held while alive, freed on death → stale record rejected |
| C14 | Lock check is a test guard, not absolute PID-reuse proof (TOCTOU) | `ax-probe.rs` `verify_fixture_lock` (checked before AX, fd closed) | documented: lock proves fixture alive at check time, not at mutation time; stronger claim needs binding fixture identity to the mutation operation (future work) |
| C15 | (M2) Explicit keyboard-first actions on the registered fixture via `ax-keyboard` | `ax-keyboard.rs` `select`/`move`/`resize`/`raise`; `fixture.rs` `verify_registration` | native run §M2: select ok · no-token exit 5 · move/resize/raise ok · invalid exit 1 · stale exit 4 |
| C16 | (M2) No loop / global hotkey / auto-tiling; raise explicit only; failure typed with rollback status + best-known frame for ALL error variants | `ax-keyboard.rs` (one-shot CLI actions, no loop); `format_ax_error` (transaction variants: own `actual` + `rollback_restored`; non-transaction: `rollback_restored=n/a` + caller-supplied frame or `<unavailable>`); `KeyboardError::Ax(AxError, Option<Rect>)`; `error_message` dispatch (propagates `best_frame` for PermissionDenied/StaleTarget; `Unregistered` reports `rollback_restored=n/a; frame=<unavailable>` precondition marker); `AxError` + `FixtureError` typed errors | native run §M2; unit tests `format_ax_error_reports_rollback_status_and_best_known_frame` (all variants) + `error_message_dispatch_propagates_best_frame` (dispatch regression incl. Unregistered); code has no background loop or hotkey daemon |

## 3. Host-bound items (cannot be verified from arena.ai)

| Item | Status | Why |
|---|---|---|
| Native AX run (C4/C5/C9/C11/C12/C13/C15/C16) | **executed on owner host**, verbatim in report §5 / M2 §Native | requires the physical Hackintosh + AX grant + self-owned target window |
| Permission-denied path | **RUNTIME TESTED (one AX-untrusted context)** | 2026-10-23 owner host: a launchd user-domain job without an AX grant ran read-only `ax-probe frame 1`; `AXIsProcessTrusted()` → false, `error: permission denied (Accessibility not granted)`, exit 3; no sudo and no TCC change (details + caveats in §6) |
| Hackintosh identity (C8) | verified read-only on owner host (kextstat, `hw.model`, EFI `SystemProductName`) | EFI partition not public; evidence table is the public record |
| Windows 11 behavior | **not verified** | no Windows boot this session; `platform-windows` deferred per `MACOS-TAHOE-NEXT-STAGE.md` |
| Multi-window writes / batch application | **native-run (best-effort rollback)** | implemented per [ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md); not atomic — a failed batch restores changed windows and reports the actual state |
| No-partial batch guarantee | **NOT MET by design** | owner accepted best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)); a failed batch may leave a partial state and says so |
| Read-only multi-window probe (2/3 fixture windows) | executed on owner host; owner-reported | same host/build as Path A; no AX writes; proves nothing about batch atomicity |

## 4. Reproduction quick start (public repo, any macOS 26 toolchain)

```
git clone https://github.com/Niumination/hypr-rust && cd hypr-rust
git checkout macos/tahoe-ax-adapter-m1

# workspace (Windows-safe)
cargo fmt --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --workspace --all-targets -- -D warnings
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace          # 23/23

# macOS-only crate (standalone)
cargo fmt --manifest-path crates/platform-macos/Cargo.toml --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --manifest-path crates/platform-macos/Cargo.toml --all-targets -- -D warnings
cd crates/platform-macos
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test                      # 37/37 lib + 2 ax-keyboard
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --bin ax-probe --bin ax-keyboard

# native (owner host): harness from spike branch @ 0eb9a25
git show spike/macos-spike-1:spikes/macos/spike-macos.swift > /tmp/spike-macos.swift
swiftc -O /tmp/spike-macos.swift -o /tmp/spike-macos && /tmp/spike-macos spawn-target &
# capture PID (pgrep) and TOKEN from ~/.hypr-rust/fixture-target.json (0600)
./target/debug/ax-probe frame <pid>                                                # read-only, no token
./target/debug/ax-probe windows <pid>                                              # read-only: enumerate AXWindows, print runtime keys
./target/debug/ax-probe set-frame --target <pid> --token <hex> 120 120 420 312      # mutate
./target/debug/ax-probe raise --target <pid> --token <hex>                          # mutate
./target/debug/ax-probe set-frame <pid> 120 120 420 312                             # exit 5 (no token)
kill <pid>; ./target/debug/ax-probe frame <pid>                                    # exit 4 (stale)

# multi-window: snapshot -> pure planner -> native-key mapping -> batch (ADR-0005)
./target/debug/ax-probe windows <pid>                                              # runtime keys for the fixture's windows
./target/debug/ax-probe apply --target <pid> --token <hex> --display 0 0 1200 800    # computed master-stack plan + apply
./target/debug/ax-probe batch --target <pid> --token <hex> --plan <file>            # explicit plan, lines "key x y width height"

# M2 keyboard-first (explicit one-shot actions on the registered fixture)
./target/debug/ax-keyboard select --target <pid> --token <hex>                      # validate fixture
./target/debug/ax-keyboard move --target <pid> --token <hex> 120 120                # explicit move
./target/debug/ax-keyboard resize --target <pid> --token <hex> 480 320              # explicit resize
./target/debug/ax-keyboard raise --target <pid> --token <hex>                       # explicit raise
./target/debug/ax-keyboard move <pid> 120 120                                       # exit 5 (no token)
```

Note: `MACOSX_DEPLOYMENT_TARGET=26.0` needed only because the owner's rustc (Homebrew 1.98.1) ships a std built for macOS 26; default x86_64 min is 10.12. Not a code issue.

## 5. Honesty constraints honored

- No serials, UUIDs, MLB, credentials, unredacted window titles, or complete EFI configs in the repo (house rule; EFI read was `SystemProductName` only).
- Permission-denied path runtime-tested in one AX-untrusted launchd user-domain context (2026-10-23, §6); Windows gap marked, not hidden.
- Spike #1 host label reconciled (was "MacBook Pro 13-inch class" → Hackintosh on ThinkPad X13 Yoga).
- Arena.ai M1 closeout packet (F1/F1b/F2) + round-4/5 reviews addressed; mapping in report §6. Mock transaction tests labelled as mock; native rollback-denial not runtime-forced. C9 softened (rollback best-effort, not atomic); C14 documents lock TOCTOU as test guard, not absolute PID-reuse proof.
- Path A (2026-10-21) and the read-only multi-window probe are **owner-reported** (owner host/build), not independently run by arena.ai.
- Native batch multi-window is implemented with best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)); it is not atomic and no all-or-nothing guarantee is claimed.
- Error map corrected: `-25208` = `notImplemented`; `-25204` = `cannotComplete`. `settable` is a per-element capability signal, not proof of runtime success, and not comparable across fixtures.

## 6. Owner-reported native evidence (2026-10-21) — Path A + read-only probe

These results were produced **on the owner's host by the owner**, not independently run by arena.ai. They are host/build-bound and are recorded as evidence, not as arena-verified claims.

**Path A — M2 native (`ax-keyboard` on the registered self-owned fixture): PASS.**
- Host: ThinkPad X13 Yoga Gen 1 (Intel i5-10310U), macOS Tahoe 26.7.1 build 25G241, AX granted, OpenCore spoof `SystemProductName=MacBookPro16,2` (a spoof is not device identity).
- `select`/`move`/`resize`/`raise` PASS (frame persisted); invalid geometry rejected **without changing the fixture** (exit 1); no-token exit 5; stale exit 4.
- **Permission-denied: runtime-tested 2026-10-23** via an AX-untrusted launchd user-domain job — exit 3 with `error: permission denied (Accessibility not granted)`; see the section below. Success on normal operations never proved behavior on failure; this run closes that gap in one context.
- Rust checks (separate): fmt OK, clippy clean, 37/37 platform-macos (+2 `ax-keyboard`), 23/23 workspace.

**Read-only multi-window probe (2 & 3 self-owned fixture windows): PASS, repeatable.**
- `AXWindows` per PID ✓; `AXUIElementGetPid` match per element ✓; `AXPosition`/`AXSize` read-back finite/positive ✓; `AXUIElementIsAttributeSettable` recorded ✓ (`AXPosition=true`, `AXSize=false`).
- **No AX writes**, no `CGWindowList`, no window titles. Proves nothing about batch atomicity.
- Error map: `-25208` = `AXError.notImplemented`; `-25204` = `AXError.cannotComplete`.
- `AXSize settable=false` came from the **probe fixture**; the M2 `resize` PASS came from a **different fixture** (registered M2 path); no write was attempted on the probe element. Cross-fixture comparison proves nothing about the same element.

**Path C — per-window runtime identity + planner wiring (macOS, host 2026-10-23): PASS.**
- `ax-probe windows <pid>` enumerated **two self-owned `HYPR-SPIKE-TARGET` windows in one process** and gave each a distinct runtime key; `ax-probe apply --display 0 24 1920 1056` computed a master-stack plan (master = the focused window), mapped planner `WindowId`s to native keys, applied both targets, `rollback_all_restored=true`, exit 0, verified by read-back; a second run was idempotent.
- `ax-probe batch --plan` applied explicit frames; a plan naming an unobserved key failed closed (`UnknownTarget`) with **no writes**.
- Coordinate convention recorded: AX frames use a **top-left origin**, so the caller-supplied work area must exclude the menu bar. `0 0 1920 1080` placed a window under the menu bar, macOS clamped it, the read-back mismatched (y 12→30) and the batch rolled back with `rollback_all_restored=true` — the rollback path verified.
- Rust: fmt OK, clippy clean, 37/37 platform-macos (+2 `ax-keyboard`), 23/23 workspace.

**Path D — minimal macOS menu-bar shell (macOS, host 2026-10-24): PASS.**
- `ax-shell` (Rust `NSStatusItem`, no new dependencies) created a status item and a 10-item menu read back through AppKit: status line, `Reflow: master-stack`, `Reflow: columns`, `Pause`, Accessibility status, `Diagnostics`, `Quit`.
- `ax-shell --self-test` ran one reflow on the registered fixture; the two windows moved from `(100,668,420,312)` / `(620,348,420,312)` to `(12,42,1038,1026)` / `(1058,42,850,1026)`, verified by `ax-probe windows`. `ax-shell` (the run loop) stayed alive.
- Auto-tiling stays off: nothing is applied until a reflow is chosen. Work area = `CGDisplayBounds` minus a documented 30 pt menu-bar inset.
- `-[NSMenuItem initWithTitle:action:keyEquivalent:]` raised an Objective-C exception on this build; items are created with `+new` plus `setTitle:`/`setAction:`/`setTarget:`.
- Not done: event loop/auto-tiling, settings UI, hotkeys, diagnostics UI (appends to `~/.hypr-rust/shell.log`), signed bundle.

**Path E — config-driven shell settings (macOS, host 2026-10-24): PASS.**
- `~/.hypr-rust/config.json` (flat JSON, no dependencies): `version`, `layout`, `outer_gap`, `inner_gap`, `master_ratio`, `auto_tiling` (default `false`), `include_dialogs`; must be a regular non-symlink owner-only `0600` file, enforced on load and on write (temp + rename). Wrong value types, out-of-range numbers and unknown layouts/versions are rejected; unknown keys are ignored.
- A `columns` config (gaps 20/4, ratio 0.6) made the menu read back `Reflow: columns` / `settings: layout=columns gaps=20/4 ratio=0.6 auto-tiling=off dialogs=out` and the reflow placed the two fixture windows at `(20,50,938,1010)` / `(962,50,938,1010)` — the exact work-area-minus-gaps arithmetic. A `0644` config failed closed with the `0600` message and fell back to defaults; `ax-shell --reset-defaults` wrote the defaults back as `0600` and a master-stack reflow reproduced `(12,42,1038,1026)` / `(1058,42,850,1026)`.
- `auto_tiling` exists as the explicit opt-in switch only; no automatic tiling is implemented and nothing is applied without a user reflow command.
- Rust: fmt OK, clippy clean, 37/37 platform-macos (+2 `ax-keyboard`), 23/23 workspace.

**Path F — per-window subrole classification + mandatory gate wiring (macOS, host 2026-10-24): PASS.**
- Enumeration reads `AXSubrole` and classifies every window before the plan: own pid → `OwnApplication` (mandatory, planner excludes unconditionally); dialog/floating subroles → `Dialog` (policy); notification subroles → `System`; `AXStandardWindow`/missing/unknown → `Application`. Runtime-observed on Tahoe 26.7.1: floating utility `NSPanel` → `AXSystemFloatingWindow`, plain `NSWindow` → `AXStandardWindow`.
- `ax-probe windows` reports `role=` per key (no titles). Default reflow with a `--dialog` fixture: `applied 2 window(s), 1 excluded`, dialog frame unchanged `(1350,465,320,224)`.
- With `include_dialogs=true` the plan covered the dialog, the floating panel **rejected the size write** (AX `-25200`) and the verified rollback restored its exact frame — the write-time gate rules, per the M3 review; the batch reported the failure honestly instead of a partial state.
- Rust: fmt OK, clippy clean, 37/37 platform-macos (+2 `ax-keyboard`), 23/23 workspace.

**Path G — settings panel over the config file (macOS, host 2026-10-24): PASS.**
- `Settings…` menu action opens an `NSWindow` panel (popup + 3 fields + 2 switches + Save/Close) built on AppKit through the existing FFI (no new dependencies). Controls populate from the effective config on every open and read back exactly on host.
- Save renders the flat JSON and runs `Config::from_json` — the same validation path as a file load — then writes atomically `0600`. A programmatic custom entry reached disk as `Columns 20/4 ratio 0.7 dialogs=true`; `master_ratio 5` was refused (`config value out of range: master_ratio`) without touching the file; defaults restored afterwards.
- Auto-tiling stays an explicit opt-in switch, off by default; nothing automatic runs.
- Rust: fmt OK, clippy clean, 37/37 platform-macos (+2 `ax-keyboard`), 23/23 workspace.

**Permission-denied native run (2026-10-23, owner host): PASS.**
- Context: a one-shot **launchd user-domain** job (`label com.hyprtest.ax-denied-check`, `RunAtLoad`, `/bin/sh` wrapper) ran the read-only `ax-probe frame 1`; the job's responsible process itself holds no Accessibility grant. No sudo, no TCC database change, no system settings or Remote Login change; the job was removed (`launchctl bootout`) immediately after and is not part of any repo.
- Result: `AXIsProcessTrusted()` returns false → `connect()` gate fires → `error: permission denied (Accessibility not granted)`, **exit 3**; no AX call was made.
- Caveat: this runtime evidence covers **one** AX-untrusted context (launchd user job). GUI prompt-flow denial and per-app revocation remain untested; do not read it as an exhaustive TCC matrix.
- Executed on the owner's host, not independently run by arena.ai.

**Gate status:** native batch multi-window application is **implemented with best-effort rollback** — see [ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md); no all-or-nothing guarantee is claimed. Permission-denied is runtime-tested in one AX-untrusted context (2026-10-23).

**Batch native runs (2026-10-22/23, owner host, same build as Path A):** `ax-probe batch` on self-owned fixture windows — A = the registered fixture; B = a second self-owned window started by the committed `spawn-target --no-register` mode of the spike harness (auxiliary target only — no fixture lock/registration; spike harness merged into `main` via PR #1 — evidence run used spike head `7636eab`). Both runs below were reproduced on 2026-10-23 with the committed harness build and produced identical results.
- CLI gates (no AX call when they fail): no args → usage, exit 1; no/bad fixture registration → exit 5; plan required to include the registered fixture and be owner-only (regular non-symlink, 0600, uid match).
- Two-window success: plan `[A, B]`, frames `(120,120,420,312)` and `(700,300,480,260)` — both applied, `rollback_all_restored=true`, exit 0; per-window `frame` read-backs matched.
- Two-window rollback: plan `[A→(40,40,300,200), B→(900,400,200,150), <live pid with no window>]`; the third target failed (`-25204 cannotComplete`) after A and B were applied; the batch stopped and restored A and B to their pre-batch frames, both verified by read-back; `rollback_all_restored=true`, exit 1.
- Single-fixture run (2026-10-22): one-target success and one failing second target, both with verified restore.
- No AX writes were made to any window other than the self-owned fixtures; no titles read.
- This is **executed on the owner's host**, not independently run by arena.ai. Batches are best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)), never atomic.
