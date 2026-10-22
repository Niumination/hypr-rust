# macOS Tahoe — Next Implementation Stage

**Owner clarification recorded:** 2026-10-18  
**Implementation focus:** user's ThinkPad X13 Yoga Gen 1, Intel i5-10310U, booted into macOS Tahoe 26.7.1. Windows 11 on the same device is deferred to a later boot/session.

## Current upstream snapshot reviewed

At review time, upstream `main` was at `d2fc8ed`; `spike/macos-spike-1` (PR #1) and `core/tiling-constraints` (PR #2) were open. Re-check branch/PR status before acting because upstream may have advanced.

- PR #1 reports AX read/move/resize/focus feasibility on a self-owned test window, and disables macOS Spaces. This matches the safe project policy.
- **Do not rerun the PR #1 test harness until the Dock mutation is removed.** Its `runTests` path attempts to set a Dock AX window position; probes must mutate only `HYPR-SPIKE-TARGET`.
- The upstream spike report calls the host “Intel MacBook Pro 13-inch class,” which conflicts with the user's ThinkPad clarification. Reconcile from `docs/SYSTEM-INFO-COLLECTION.md`; do not silently carry forward the old machine label.
- Existing macOS workspace result is `DISABLED`; keep it disabled. No private CGS/SkyLight interface or hide/minimize substitute.
- PR #2 adds min-size constraints; review/merge it separately from native adapter work.

## Product/support boundary for this stage

- The immediate purpose is to validate and implement a macOS AX window-controller on the user's **specific current host and OS build**.
- A pass here is not proof of support for Apple-branded Macs, Ventura 13, all Intel Macs, or all macOS versions.
- Windows is not being dropped: keep the shared core platform-neutral, avoid macOS-only domain assumptions, and preserve a later Windows adapter path. Do not build/test Windows native operations in this stage.
- No app UI or broad all-window automation until the narrow adapter is safe, repeatable and reviewed.
- The user's host is a ThinkPad, not Apple-branded hardware. macOS Tahoe's published license is headed “For use on Apple-branded Systems”; do not make legal/support claims about this configuration without review. This is a product/legal caveat, not a technical instruction to alter the OS.

## Stage M1 — Rust AX adapter proof (macOS-only vertical slice)

### Before implementation

1. Agent reads root `AGENTS.md`, `docs/PRD.md`, `docs/ARCHITECTURE.md`, `docs/adr/0003-native-workspaces.md`, this note and the current upstream spike files.
2. Agent reports branch/commit, `cargo` baseline, host info from `scripts/collect-macos-env.sh`, Accessibility trust status and an implementation plan.
3. Fix/remove the Dock mutation in the existing Swift spike in a dedicated reviewed commit before using it as a reference again.
4. Resolve exact hardware identity from system output. Do not include serial, hardware UUID, EFI config, or raw NVRAM data.

### Scope

Implement a small **Rust** macOS adapter/probe behind the existing platform-neutral core boundary. It should use reviewed public Accessibility/CoreGraphics APIs and be runnable from the user's current Tahoe environment.

- Start with one self-owned `HYPR-SPIKE-TARGET` application window and explicit Accessibility permission.
- Discover/read a target window through the chosen public API path; do not enumerate or print user titles in the test report.
- Apply move/resize only to the explicit test target; verify by reading the resulting frame.
- Make focus/raise a separate explicit test; do not steal focus during passive discovery.
- Represent denied permission, unsupported attributes, stale element/process, and OS refusal as distinct errors/results.
- No Dock/Finder/system UI mutation; no global auto-tiling; no global event loop; no background startup; no workspace code; no private APIs.
- Keep `unsafe` FFI confined to a small adapter module with safety comments; platform handles must not enter the shared domain or persist across process lifetimes.
- Prefer Rust for production code. Swift is acceptable only for the existing disposable spike/behavior comparison; do not quietly turn the runtime into a Swift helper unless a new ADR explicitly approves that architecture.

### Acceptance criteria

1. Baseline `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass on the exact host; attach toolchain versions and output.
2. A self-owned window can be discovered, its frame read, moved and resized repeatedly, then read back within a documented tolerance.
3. The probe mutates only its own target; code search and execution record confirm no system/user window mutation.
4. Accessibility denied path is explicitly exercised or accurately marked “not runtime tested”; do not call a code branch an observed pass.
5. Stale target/window failures return promptly without crash, hang, retry loop or false success.
6. Logs contain no window titles, serials, UUIDs, credentials or raw system configuration.
7. Record exact host/OS and keep native workspaces explicitly disabled. Do not claim support on other Macs/OS versions.

### Stop/go

- **Go to macOS app shell:** M1 passes and unsafe FFI/API review is clear. Then add a minimal menu-bar shell and opt-in settings, still with a pause/quit path.
- **Hold:** permission identity, system/user window filtering, coordinate conversion, stale handles, or geometry writes are uncertain. Resolve with targeted probes first.
- **Workspace:** remains `DISABLED` on macOS for this stage. No attempt to bypass that decision.

## Agent task prompt

```text
Work only on the macOS Tahoe M1 vertical slice described in
`docs/MACOS-TAHOE-NEXT-STAGE.md`.

First inspect the repo and report exact branch/commit, toolchain and host info.
Do not run the existing macOS spike's Dock mutation; remove it before any rerun.
Use a self-owned test window only. Implement the smallest Rust AX adapter proof
with reviewed public APIs. No private APIs, workspace/Spaces logic, global
window management, title logging, startup service, or UI yet.
Run fmt/clippy/tests and native move/resize/stale/permission tests on the host.
Report each result and limitation honestly. A later macOS build/app must not be
claimed supported beyond the exact hardware/OS tested here.
```

## Later Windows preparation (not this stage)

Keep the core `WindowId`, `Rect`, layout plans, errors and tests independent of Apple frameworks. When the device next boots Windows 11, run a separate Windows 11 x64 discovery/move/resize spike and add `platform-windows` behind `cfg(windows)`. Do not share native handles or claim desktop/workspace support from the macOS result.
