# Delivery Roadmap

**Planning basis:** interview decisions captured 2026-10-17. Estimates remain relative; do not publish calendar commitments before spikes, staffing and scope are agreed.

## Current status (2026-11-04)

- **Pure layout planning is complete and tested** — `layout-core` ships all five layouts (BSP, master/stack, monocle, columns, rows) plus rules/exclusions and clamp-aware geometry; no OS side effects.
- **Real-window reflow is implemented as strict no-partial and owner-validated on real windows** ([ADR-0006](./adr/0006-real-window-reflow-strict-no-partial.md)): preflight read-before-write, verified writes, full rollback, honest three-way status — host-validated on all three legs 2026-10-27, and the clamp-skip amendment §2.5 host-verified 2026-11-03 (ARENA Path O, four host runs). The **fixture-bound** self-test path keeps best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md), which supersedes [ADR-0004](./adr/0004-batch-multiwindow-deferred-no-partial.md)) and never claims atomicity.
- **Global hotkey `⌥⌘R`** ([ADR-0007](./adr/0007-global-hotkey-reflow.md), Path P host-verified 2026-10-28) and **opt-in event-driven auto-tiling** ([ADR-0008](./adr/0008-auto-tiling-event-driven.md), Path Q host-verified 2026-10-30/31) are implemented; auto-tiling stays off until the user enables it explicitly.
- **All native evidence is host-bound:** ThinkPad X13 Yoga Gen 1 (Intel i5-10310U), macOS Tahoe 26.7.1 build 25G241, single 1920×1080 display. It does **not** validate Apple hardware or older macOS; packaging is a local unsigned `.app` (signing/notarization open).
- **Windows 11** waits for a Windows boot; the Windows adapter is not implemented and there is no Windows runtime evidence yet.
- **No cross-platform parity is claimed.** macOS Spaces remain disabled (no public API proven for the required set). Permission-denied is runtime-tested in one AX-untrusted launchd user-domain context (2026-10-23).

## Phase 0 — feasibility and product boundary

**Exit:** ADR-0001/0002/0003 accepted; target OS matrix is Windows 11 x64 and Intel macOS Ventura 13+; public API feasibility is known for core window control and optional native workspace operations.

- Run parallel native probes on the available Windows 11 and Intel Mac machines.
- Validate enumeration, observation, move/resize, hotkey registration, lifecycle, permissions, signing identity and direct packaging.
- For workspace operations, test the complete required set: detect active/target desktop/space, switch to a selected desktop/space, move the focused window to a selected desktop/space, observe changes and report denials. Record build-specific evidence.
- If an OS cannot meet the workspace contract through reviewed public APIs, disable workspace controls on that OS. Do not substitute minimize/hide or private APIs.
- Confirm settings UI, Windows tray and macOS menu-bar approach; verify installers and signing/notarization path.
- Review dependency licenses and public product name.

**Current sequencing (2026-11-04):** only the Tahoe host has been exercised — single-window M1/M2 via public AX (2026-10-22), then the full real-window path: strict reflow ([ADR-0006](./adr/0006-real-window-reflow-strict-no-partial.md), host-validated 2026-10-27 + clamp-skip 2026-11-03), global hotkey (Path P) and opt-in auto-tiling (Path Q). The Windows machine has not been booted in this effort. The Ventura/Apple-Mac target and the parallel-probe items above remain **unverified future targets**, not tested claims. The **fixture-bound** batch path uses best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)); the planner was never the blocker.

**Stop/go:** If ordinary window control is unreliable on one OS, revise supported-window claims before implementation. Workspace feasibility failure alone does not stop the cross-platform tiling core.

## Phase 1 — shared engine proof

**Exit:** deterministic layout core passes property/golden tests without OS dependencies.

**Note (2026-10-22):** this phase may proceed — it has no OS side effects and is independent of the native batch gate ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)).

- Define coordinate model, runtime IDs, snapshots, mandatory exclusions, capability model, plan/report types.
- Implement BSP and master/stack first, then monocle and columns/rows; document minimum-size/rounding behavior.
- Build a fake adapter and settings schema/UI state model.
- Benchmark synthetic states; prove plans are idempotent and failures do not trigger loops.

## Phase 2 — parallel OS vertical slices

**Exit:** dev builds on both OS can discover ordinary windows, apply one layout, pause/quit safely and show diagnostics.

### Windows 11 x64
- Tray lifecycle, settings UI, platform-default hotkeys and conflict reporting.
- Window/display discovery, event handling, DPI/multi-monitor correctness.
- Validate elevated targets and focus restrictions; no default elevation.

### Intel macOS Ventura 13+
**Updated (2026-11-04):** the macOS vertical slice on Tahoe is implemented and host-validated — menu-bar shell + settings/rules editor, strict real-window reflow ([ADR-0006](./adr/0006-real-window-reflow-strict-no-partial.md)) with clamp-skip, global hotkey `⌥⌘R` ([ADR-0007](./adr/0007-global-hotkey-reflow.md)) and opt-in event-driven auto-tiling ([ADR-0008](./adr/0008-auto-tiling-event-driven.md)); the fixture-bound batch keeps best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)). Packaging is a local unsigned `.app`; the Ventura 13+ target remains unverified.
- Menu-bar lifecycle, settings UI, Accessibility explanation/request and revocation handling.
- AX discovery/events, public display APIs and coordinate conversion.
- Validate signed identity/trust, representative apps and permission failures.

### Shared requirements
- Auto-tiling only after explicit onboarding opt-in.
- Mandatory safety exclusions; user-configurable ordinary window rules.
- Direct-download packaging prototypes; code signing/notarization path tested. *(2026-10-26: prototype started — local unsigned `.app` assembled by `scripts/package-macos.sh` and attached as a CI artifact; the signing/notarization path itself remains open.)*

## Phase 3 — native workspace capability decision

**Exit:** per-platform capability matrix and explicit enabled/disabled decision.

- Integrate workspace shortcuts only where all required public-API operations are verified on the declared OS builds.
- If unavailable, disable workspace actions on that platform and explain why; do not affect layout management.
- A denied per-window operation leaves that window unchanged and is reported; no minimize/hide fallback.
- At startup follow the OS-selected desktop/space; do not force a switch.

## Phase 4 — MVP hardening and beta

**Exit:** P0 stories work on both platforms; workspace capability is accurately gated; beta test matrix passes.

- Complete four initial layout families, settings, rules, hotkey conflicts, pause, capability diagnostics and safe config reload.
- Add real-app tests selected for developer and general desktop workflows.
- CI, fuzzing, privacy/legal review, performance/soak tests and direct signed artifacts.
- Collect beta feedback and reproducible diagnostics.

## Phase 5 — stable release and maintenance

- Publish signed/notarized direct downloads with checksums, SBOM/license notices, support policy, uninstall and rollback instructions.
- Track Windows/macOS changes; test on new builds before claiming support.
- Revisit workspace availability per OS only with new evidence; no parity promise by default.
- Consider workspaces beyond native OS semantics, overlay, package managers or Linux only through new product decisions/ADRs.

## Suggested ownership

- **Tech lead:** architecture, API contracts, ADRs, release quality.
- **Rust/core engineer:** domain, layouts, config/settings model, reducer and property tests.
- **Windows engineer:** Win32 behavior, events, DPI and packaging.
- **macOS engineer:** AX, permissions/signing, display geometry and packaging.
- **QA/security:** native matrix, privacy threat model, integration/soak and release checks.

One engineer may cover multiple roles; unsafe FFI and permission-sensitive work require explicit review.

## Immediate next actions

**Validated (through 2026-11-04; all owner-reported and host-bound to the ThinkPad/Tahoe build above unless noted):**

1. Pure layout Phase 1 — `layout-core` five layouts, rules/exclusions and clamp-aware geometry, workspace tests green (no OS side effects).
2. macOS Tahoe single-window M1/M2 (AX adapter + `ax-keyboard` select/move/resize/raise), 2026-10-22.
3. Fixture-bound native batch two-window success + rollback, plus one permission-denied runtime test (AX-untrusted launchd context, 2026-10-23) — [ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md).
4. Real-window strict reflow across master-stack/monocle/bsp/columns/rows — three legs host-validated 2026-10-27, clamp-skip host-verified 2026-11-03 — [ADR-0006](./adr/0006-real-window-reflow-strict-no-partial.md).
5. Global hotkey `⌥⌘R`, host-verified 2026-10-28 — [ADR-0007](./adr/0007-global-hotkey-reflow.md).
6. Opt-in event-driven auto-tiling (open/close → one coalesced strict re-tile), host-verified 2026-10-30/31 — [ADR-0008](./adr/0008-auto-tiling-event-driven.md).
7. End-to-end multi-window layout on macOS — **done** (strict no-partial, not best-effort); Windows pending a boot.
8. CI on GitHub `macos-26` (fmt/clippy/tests + package) green through PR #47; local unsigned `.app` bundle path proven (signing/notarization still open).

**Planned (not yet validated):**

1. **Operator/production hardening** — `.app` LaunchServices/TCC re-validation after each rebuild, signing/notarization or an accountable distribution strategy, Accessibility-permission guidance under the bundled identity, install/update, and clear first-run auto-tiling opt-in.
2. **Hardware/display matrix** — **host-scoped slice done 2026-11-07** ([HARDWARE-DISPLAY-MATRIX](./HARDWARE-DISPLAY-MATRIX.md): read-only display probe + unit rows for fullscreen/minimized/off-display/work-area + honest NOT RUN rows); repeat on a target Apple Mac; multi-display/mixed-DPI, display/work-area changes and varied apps still pending hardware; document unsupported cases without moving or hiding windows.
3. **Min-size/fit policy for dense layouts** — clamp-skip stays honest (a skipped window is not tiled); decide whether min-size-aware planning is worth it.
4. Windows 11 feasibility spike + adapter — waits for a Windows boot; no Windows runtime evidence yet.
5. macOS Ventura 13+ / Apple-Mac validation — future target, still unverified.
6. Define the exact workspace operation checklist and capture a go/no-go result per OS (Spaces stay disabled until every required operation is proven through public APIs).
7. Select representative apps for developer and general-use validation.
8. Update the supported-capability matrix from test evidence before adding workspace shortcuts to UI.
9. Extension ecosystem (after the Mac core stabilizes) — owner-selected versioned local Unix-socket IPC, native status bar/widget API and a separate opt-in capability model; no Waybar/Hyprland binary-compatibility promise. ADR-0009 remains Proposed. Read-only `status.get` shipped upstream via PR #49 and is host-validated on seven required checks (ARENA Path R, 2026-11-07); other-UID peer gate and second-instance bind-conflict remain NOT runtime-tested. Next: protocol/schema review, then a native bar consumer — action capabilities only as a separate owner decision.