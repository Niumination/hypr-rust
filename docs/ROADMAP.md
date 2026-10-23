# Delivery Roadmap

**Planning basis:** interview decisions captured 2026-10-17. Estimates remain relative; do not publish calendar commitments before spikes, staffing and scope are agreed.

## Current status (2026-10-22)

- **Pure layout planning may continue** (`layout-core::compute_layout` with `LayoutKind::MasterStack` and its tests; no OS side effects).
- **Native batch multi-window application is implemented with best-effort rollback** — the owner changed the semantics (strict no-partial is no longer required); see [ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md), which supersedes [ADR-0004](./adr/0004-batch-multiwindow-deferred-no-partial.md). A failed batch restores every already-changed window and reports the actual state honestly; it never claims atomicity.
- **Native macOS M2 (Jalur A) PASS is host-bound:** ThinkPad X13 Yoga Gen 1 (Intel i5-10310U), macOS Tahoe 26.7.1 build 25G241, self-owned fixture window. It does **not** validate Apple hardware or older macOS.
- **Windows 11** waits for a Windows boot; no Windows runtime evidence yet.
- **No cross-platform parity is claimed.** Permission-denied is runtime-tested in one AX-untrusted launchd user-domain context (2026-10-23).

## Phase 0 — feasibility and product boundary

**Exit:** ADR-0001/0002/0003 accepted; target OS matrix is Windows 11 x64 and Intel macOS Ventura 13+; public API feasibility is known for core window control and optional native workspace operations.

- Run parallel native probes on the available Windows 11 and Intel Mac machines.
- Validate enumeration, observation, move/resize, hotkey registration, lifecycle, permissions, signing identity and direct packaging.
- For workspace operations, test the complete required set: detect active/target desktop/space, switch to a selected desktop/space, move the focused window to a selected desktop/space, observe changes and report denials. Record build-specific evidence.
- If an OS cannot meet the workspace contract through reviewed public APIs, disable workspace controls on that OS. Do not substitute minimize/hide or private APIs.
- Confirm settings UI, Windows tray and macOS menu-bar approach; verify installers and signing/notarization path.
- Review dependency licenses and public product name.

**Current sequencing (2026-10-22):** only the Tahoe host has been exercised so far — single-window M1/M2 via public AX (`#1`, `#3`, `#4`). The Windows machine has not been booted in this effort. The Ventura/Apple-Mac target and the parallel-probe items above remain **unverified future targets**, not tested claims. Native **batch multi-window** uses best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)); the planner was never the blocker.

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
**Updated (2026-10-22):** native batch multi-window application is implemented with best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)); current Tahoe evidence is single-window M1/M2 plus native batch runs (two-window success and two-window rollback). The Ventura 13+ target remains unverified.
- Menu-bar lifecycle, settings UI, Accessibility explanation/request and revocation handling.
- AX discovery/events, public display APIs and coordinate conversion.
- Validate signed identity/trust, representative apps and permission failures.

### Shared requirements
- Auto-tiling only after explicit onboarding opt-in.
- Mandatory safety exclusions; user-configurable ordinary window rules.
- Direct-download packaging prototypes; code signing/notarization path tested.

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

**Validated (2026-10-22):** macOS Tahoe single-window M1/M2 (AX adapter + `ax-keyboard` select/move/resize/raise) on the owner's ThinkPad X13 Yoga Gen 1, macOS Tahoe 26.7.1 build 25G241 — owner-reported, host-bound; does not cover Apple Mac or Ventura. Native batch multi-window is **implemented with best-effort rollback** ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)) and was native-run on the same host.

**Planned (not yet validated):**

1. Windows 11 feasibility spike — waits for a Windows boot; no Windows runtime evidence yet.
2. macOS Ventura 13+ / Apple-Mac validation — future target, still unverified.
3. Pure layout Phase 1 (shared geometry/layout core; BSP/master-stack) — may proceed now, no OS side effects.
4. Define the exact workspace operation checklist and capture a go/no-go result per OS.
5. Select representative apps for developer and general-use validation.
6. One end-to-end multi-window layout per OS — macOS done with best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)); Windows pending a boot.
7. Update the supported-capability matrix from test evidence before adding workspace shortcuts to UI.