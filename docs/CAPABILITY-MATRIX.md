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
| Settings panel + **row-based rules editor** (Save canonical JSON; click-path `performClick:` proof) | PASS | Paths J → L → M |
| Reflow on two fixture windows (master-stack/columns) | PASS fixture-bound | ARENA Path K/L/M self-tests |
| Config safety (owner-only 0600, symlink rejection, fail-closed classification) | PASS 4-case + 2-case hosts | ARENA Jalur K addendum 2026-10-25 |
| **Spaces / native workspaces (macOS)** | **DISABLED** — requirements not proven on this build; no private WindowServer use | [ADR-0003](./adr/0003-native-workspaces.md) |
| Global hotkeys | NOT IMPLEMENTED | docs/README outstanding list |
| **Packaging** | **LOCAL DEV BUNDLE ONLY** — `scripts/package-macos.sh` → unsigned/ad-hoc `.app`, CI artifact on `macos-26`; DMG + signing/notarization NOT delivered (PRD β-release gate pending proven plan); distribution claims pending legal review (non-Apple host, USER-DEVICE-PROFILE §5); **AX trust under bundled identity NOT runtime-tested** | [USING-AX-SHELL](./USING-AX-SHELL.md) |

## Windows 11 x64

| capability | state |
|---|---|
| Adapter (Win32 enumerate/request placement) | NOT IMPLEMENTED — no native run exists; nothing validated |
| Everything else in this matrix | NOT VALIDATED — no native evidence on any Windows build |

## Build hosts (not runtime validation)

CI (GitHub `macos-26` images, SDK 26, toolchain 1.98.1) runs fmt/clippy/tests and the packaging script on **both** jobs. A green CI is a compile/package-path check, **not** runtime validation — runtime rows above are host-bound only.
