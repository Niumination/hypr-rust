# OpenCode / Hermes Agent Handoff Plan

**Current workflow:** the upstream repository already exists at `https://github.com/niumination/hypr-rust`. The user's current development host is a ThinkPad X13 Yoga Gen 1 (Intel i5-10310U) dual-booting Windows and macOS Tahoe 26.7.1. Work is macOS-first; Windows is deferred until the next Windows boot.

## Repository facts from the latest review

At the time of the review, `main` was at `d2fc8ed`; macOS spike PR #1 and core tiling constraints PR #2 were open. Re-check upstream before starting so the agent uses the latest base commit.

- PR #1 contains a Swift public-API spike and reports AX control PASS on a self-owned test window; native Spaces are correctly disabled. **Do not rerun its current `run-tests` path until the Dock mutation is removed and reviewed.**
- The spike report labels the host “Intel MacBook Pro 13-inch class”, conflicting with the user's ThinkPad clarification. Use the read-only system collector and amend the report with the verified exact host description.
- PR #2 contains platform-neutral min-size constraints and reports 13 tests passing. Keep it isolated from macOS adapter changes unless the base is deliberately updated.
- The current repo is not a finished desktop app; no native app shell or production adapter has been verified.

## Current stage: macOS Tahoe M1

Read `docs/USER-DEVICE-PROFILE.md`, `docs/MACOS-TAHOE-NEXT-STAGE.md`, `docs/SYSTEM-INFO-COLLECTION.md`, and `AGENTS.md` before editing.

1. Create a read-only environment report using `sh scripts/collect-macos-env.sh`; reconcile the host identity in the upstream macOS report. Do not include serial numbers, UUIDs, EFI/OpenCore config, window titles or credentials.
2. First fix the existing Dock mutation in a dedicated commit. A spike may modify only its self-owned `HYPR-SPIKE-TARGET`; never Dock, Finder, other system UI or arbitrary user windows.
3. Run baseline `cargo fmt`, `cargo clippy`, and `cargo test` on the exact host; report toolchain and exact output.
4. Build the smallest Rust AX adapter proof described in `docs/MACOS-TAHOE-NEXT-STAGE.md`: discover/read/move/resize a self-owned target; typed permission/stale/unsupported failures; no UI, startup service, global auto-tiling, Spaces or private APIs.
5. Test on the actual macOS Tahoe 26.7.1 host and report limitations. Do not claim Ventura or Apple-branded Mac support from this result.
6. Submit as a focused PR with exact base/head SHAs and reproducible report. Update docs/capabilities in the same PR.

Suggested branch: `macos/tahoe-ax-adapter-m1` (or another clearly scoped name).

## Deferred: Windows 11 on the same device

When the user next boots Windows 11, create a separate `spike/windows11-x64-window-control` branch. Record exact Windows build, Rust toolchain, display/DPI setup and native results. Keep Windows APIs under `cfg(windows)` and do not infer Windows behavior from the macOS adapter. Native Windows workspaces remain unclaimed until separately tested.

## PR review checklist

For each PR require:

- Exact base/head commit SHAs, branch, file list and clean/dirty working tree status.
- Exact physical host as reported by the OS, OS build, permissions and app identity.
- Exact toolchain, test commands and outputs; native tests run on the target OS.
- For window operations, name the self-owned test target and show only redacted geometry/status evidence.
- Explicit status for each case: pass, fail, unsupported, not tested, or inconclusive. Do not translate “code path exists” into “runtime verified.”
- No mutation of system/user windows, private APIs, privilege escalation, hidden/minimize fallback, or unreviewed dependencies.

## Prompt for the current macOS agent

```text
Read AGENTS.md, README.md, docs/USER-DEVICE-PROFILE.md,
docs/MACOS-TAHOE-NEXT-STAGE.md, docs/SYSTEM-INFO-COLLECTION.md,
docs/ARCHITECTURE.md, and docs/adr/0003-native-workspaces.md.

Task: macOS Tahoe 26.7.1 M1 Rust AX adapter proof on the owner's ThinkPad X13 Yoga Gen 1.
First report exact base SHA, branch, system collector output (redacted), toolchain,
and baseline test results. Resolve the old spike report's incorrect/uncertain host label.
Do not run the old Dock mutation; remove it before rerunning that harness.
Implement only a narrow Rust public-AX proof against a self-owned HYPR-SPIKE-TARGET.
No Dock/Finder/user-window mutations, window-title logs, workspace/Spaces, private APIs,
background startup, broad auto-tiling or UI yet. Keep native handles/unsafe code local.
Exercise move/resize, stale target and actual permission-denied behavior; report anything
not tested honestly. Run fmt/clippy/tests and the native probe on the exact host.
Submit a focused PR with evidence and known limitations; do not claim other Mac hardware
or OS versions are supported.
```

## Verification loop with this reviewer

After the agent pushes, share the upstream PR URL and exact head SHA. Review will compare the exact diff, reported machine/system data and test evidence. Public links are sufficient for public branches; for private material provide an exported patch and redacted logs. Never share tokens, credentials, serial numbers or user-window data.
