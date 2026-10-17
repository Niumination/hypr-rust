# User Device Profile & Validation Scope

**Last updated:** 2026-10-18  
**Source:** user-provided clarification; verify the exact values from the system before treating this as test evidence.

## Current implementation host

- **Physical device (user-reported):** Lenovo ThinkPad X13 Yoga Gen 1
- **CPU (user-reported):** Intel Core i5-10310U
- **OS setup:** dual-OS on the same physical device; Windows is a separate boot, not a VM
- **Current development/test OS (user-reported):** macOS Tahoe 26.7.1
- **Immediate priority:** macOS window-management vertical slice on the user's current macOS installation
- **Deferred for now:** Windows native adapter and Windows runtime spike; retain portable architecture and prepare Windows task/docs, but do not block macOS progress on them

These are currently owner-reported facts, not independently collected evidence. The macOS spike report in the upstream repo describes the host as an “Intel MacBook Pro 13-inch class”; that conflicts with this clarification. Reconcile the report using the exact system model/identifier collected by `scripts/collect-macos-env.sh` before the report is treated as reproducible hardware evidence.

## Scope distinction

1. This machine is the **current user test host**, not an Apple-branded Mac. Passing on it proves behavior only on this reported host/OS setup.
2. Do not infer compatibility with all Intel Macs, Apple-branded hardware, Ventura 13, or earlier/later macOS versions from this one host.
3. The prior macOS spike reports ordinary AX control on a self-created test window and disables native Spaces. Keep the workspace feature disabled on macOS unless a new public-API proof meets the complete contract. Do not spend implementation time trying private CGS/SkyLight APIs or a hide/minimize fallback.
4. Windows remains a planned target. Its native behavior is unverified until the laptop boots Windows 11 and the Windows spike is run there.
5. Distribution/support positioning for macOS on non-Apple hardware is an unresolved legal/product question. Apple's published macOS Tahoe license is headed “For use on Apple-branded Systems”; obtain appropriate legal review before describing this setup as a supported distribution target. This note is not legal advice. [Apple macOS Tahoe Software License Agreement](https://www.apple.com/legal/sla/docs/macOSTahoe.pdf)

## Immediate macOS implementation goal

Build a **small Rust macOS adapter proof**, not the full desktop UI yet:

- Use the existing Swift probe only as a behavioral reference; review it before reuse.
- First remove the Dock-position mutation from the spike harness. Never mutate Dock, Finder, system UI, or arbitrary user windows in a probe.
- Use a self-owned test window (`HYPR-SPIKE-TARGET`) and explicit Accessibility permission.
- Implement/read-test a narrow Rust boundary for window discovery, position/size read/write, focus request if supported, and typed denied/stale/unsupported results.
- Verify permission denial and stale element handling. Do not silently convert an API error to success.
- Keep window titles out of logs. Use app identity only when necessary and redact it in reports.
- Validate on the exact host and record the current report using `docs/SYSTEM-INFO-COLLECTION.md` and `docs/PLATFORM-SPIKES.md`.
- Do not claim a distributable app or all-app compatibility from the probe. A tray/menu-bar shell and settings UI follow after the native adapter and event lifecycle are reviewed.

## Per-session report fields

Every agent report must include:

- Current branch and full commit SHA; `git status --short`.
- Exact `sw_vers` version/build, architecture, CPU and system-reported model identifier.
- Rust, Cargo, Swift and Xcode Command Line Tools versions/paths.
- Display count, frame/visible frame and scale (no serial numbers or screenshots needed).
- Which process was granted Accessibility trust (Terminal, Swift probe or signed app); permission state must not be assumed to transfer between process identities.
- Exact commands and results for formatting, clippy, tests and each native operation.
- Failures and limitations (including single-display, unsupported-app and Permission-denied cases).

Never include serial numbers, hardware UUIDs, Apple IDs, tokens, passwords, complete EFI/OpenCore configs, unredacted window titles, or raw `nvram`/`ioreg` dumps in the repository.