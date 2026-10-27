# Capability matrix — per platform / OS build

PRD requirement ("publish a capability matrix per platform/OS build; do not claim support for apps/windows that fail capability checks"). Rows are the product capabilities; evidence columns link the audited native runs. Labels: **PASS** (native-verified), **HONEST-REFUSAL** (documented failure semantics, not a support claim), **DISABLED** (gated off by decision), **NOT IMPLEMENTED**, **NOT VALIDATED**.

## macOS (owner's host: ThinkPad X13 Yoga Gen 1 Hackintosh, macOS Tahoe 26.x · builds: pinned toolchain 1.98.1, deployment target 26.0)

| capability | state | evidence |
|---|---|---|
| Single-window AX read/move/resize/raise (M1) | PASS (host-bound) | [M1 report](./reports/MACOS-M1-AX-ADAPTER-2026-10-18.md); ARENA Paths A–B |
| Keyboard-first explicit control (M2) | PASS (host-bound) | `ax-keyboard` bin, Jalur A (ARENA-VERIFICATION) |
| Multi-window batch, best-effort + rollback | PASS fixture-bound; **never claimed as no-partial success** | ARENA Paths F–H; [ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md) |
| Dialog/floating panel exclusion & size-write refusal | PASS / HONEST-REFUSAL (`-25200` size write after position, per-window rollback restored) | PLATFORM-SPIKES dialog section, ARENA Path J (include+dialogs=off round) |
| Read-only enumeration/geometry probe | PASS (read-only; not control evidence) | [PLATFORM-SPIKES](./PLATFORM-SPIKES.md) windows section |
| Accessibility permission denial path | PASS one launchd context (exit 3) | 2026-10-23 runtime test, ARENA §6 caveat |
| Menu-bar shell: status, reflow, pause, diagnostics, quit | PASS | ARENA Path J / host run 2026-10-24 |
| Settings panel + **row-based rules editor** (Save canonical JSON; click-path `performClick:` proof; per-field number validation + in-panel status line) | PASS | Paths J → L → M |
| Reflow on two fixture windows (master-stack/columns) | PASS fixture-bound | ARENA Path K/L/M self-tests |
| **Real-window reflow** (menu command: all normal windows of every regular app, strict no-partial, rules app protection) | **PASS host-run (success + refusal), 2026-10-27** — owner runs: `tiled 3`/`tiled 4 window(s)` on master-stack, monocle and bsp (every success = per-window read-back verified); refusals always clean — busy apps (`-25204`, zero writes), app min-size clamps (Nicegram 628 / Finder 511×252 / Firefox 500 → read-back mismatch → full verified rollback, status names `failed at app pid …`); rules app protection used live (exclude clamping apps, re-include Finder where it fits). ADR-0006 §2.1 retry/skip amended same day (no busy-app run of its own yet) | [ADR-0006](./adr/0006-real-window-reflow-strict-no-partial.md); ARENA Path O; unit evidence 71 lib + 29 workspace tests |
| Config safety (owner-only 0600, symlink rejection, fail-closed classification) | PASS 4-case + 2-case hosts | ARENA Jalur K addendum 2026-10-25 |
| **Spaces / native workspaces (macOS)** | **DISABLED** — requirements not proven on this build; no private WindowServer use | [ADR-0003](./adr/0003-native-workspaces.md) |
| Global hotkeys | **IMPLEMENTED** — one chord **⌃⌥⌘R** → the same `perform_reflow_real` as the menu, registered once at shell startup via Carbon `RegisterEventHotKey` (public API, no new TCC permission); fail-soft conflict report (`eventHotKeyExistsErr -9878` named in the log, menu keeps working); automated paths never register it; unit tests 75 lib | [ADR-0007](./adr/0007-global-hotkey-reflow.md); host press evidence pending (owner run) |
| **Packaging** | **LOCAL DEV BUNDLE ONLY** — `scripts/package-macos.sh` → unsigned/ad-hoc `.app`, CI artifact on `macos-26`; DMG + signing/notarization NOT delivered (PRD β-release gate pending proven plan); distribution claims pending legal review (non-Apple host, USER-DEVICE-PROFILE §5); **AX trust under the bundled identity runtime-tested** (Path N): LaunchServices identity denied until an explicit TCC grant, then granted with fixture reflow applied — direct-exec inherits the CLI-era chain grant | [USING-AX-SHELL](./USING-AX-SHELL.md); ARENA Path N |

## Windows 11 x64

| capability | state |
|---|---|
| Adapter (Win32 enumerate/request placement) | NOT IMPLEMENTED — no native run exists; nothing validated |
| Everything else in this matrix | NOT VALIDATED — no native evidence on any Windows build |

## Build hosts (not runtime validation)

CI (GitHub `macos-26` images, SDK 26, toolchain 1.98.1) runs fmt/clippy/tests and the packaging script on **both** jobs. A green CI is a compile/package-path check, **not** runtime validation — runtime rows above are host-bound only.
