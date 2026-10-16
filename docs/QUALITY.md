# Quality, Testing & Release Plan

## 1. Quality goals

- Correctness of geometry and state transitions is proven independently from OS calls.
- Platform failures are visible, typed and recoverable; no false success.
- Auto-tiling does not start until the user explicitly opts in during onboarding/settings; permission grant alone is insufficient.
- A single broken window cannot destabilize all managed windows.
- Release claims match tested OS versions and permission conditions.

## 2. Test pyramid

### Unit tests (every PR)

- Layout invariants across randomized window counts, display sizes, ratios, gaps and minimum sizes (property-based tests).
- Golden test cases for BSP insertion/removal, master-stack, columns/rows, monocle, float/ignore rules, stable ordering and rounding.
- Geometry conversion tests for negative display origins, mixed scale, edge touching and zero-sized/invalid input.
- Config parsing, defaults, schema-version migrations and invalid/unknown fields.
- Reducer lifecycle: create/destroy/reuse, stale events, permission change, own-write suppression, event storms.

### Component tests (every PR)

- Fake `PlatformPort` verifies plan ordering, cancellation, bounded retries, partial apply reports, shutdown behavior and that windows remain at last accepted geometry (no initial-geometry restoration).
- Event coalescer verifies event loss reconciliation and no unbounded queue growth.
- Config hot reload proves invalid candidate config does not replace last-known-good state.

### OS integration tests (nightly/release candidate)

- Test app windows: normal, owned dialog, tool window, minimized, maximized, fullscreen, topmost, elevated (Windows), inaccessible AX window (macOS).
- Spawn/move/resize/close rapidly; monitor actual geometry and focus; revoke permissions during run.
- Multi-monitor layouts, negative coordinates, hotplug, mixed DPI/scale changes, display sleep/wake.
- Test real apps across browsers, IDEs, terminals and office apps, with explicit opt-in. Do not make brittle assumptions from a single app.

### Native workspace capability tests (per OS/build; optional feature gate)

- Verify the complete contract using public APIs: detect active/target desktop/space, switch to a selected desktop/space, move the focused window to a selected desktop/space, observe changes and report denials.
- Record OS build, app signing identity, permission state, operation result and reproducible evidence.
- If required operations fail or use private APIs, workspace actions remain disabled on that platform; core tiling release is not blocked.
- A denied per-window operation leaves the window unchanged and reports a capability/error; test that no minimize/hide fallback runs.
- Startup follows the OS-selected desktop/space; app does not force a switch.

### Manual acceptance

- First-run permission flow; auto-tiling remains off until explicit opt-in.
- Shortcut conflicts and emergency pause/quit.
- Tray/menu-bar lifecycle, startup behavior, direct installer/update/uninstall, settings recovery.
- Screen reader and keyboard-only operation for all app UI.

## 3. Test environments

Maintain reproducible test records for:

- Windows 11 x64: exact build, DPI settings, GPU/display setup, privilege level and test apps; workspace feature tested and gated independently.
- Intel macOS Ventura 13+: exact version/build, Intel model, signing identity, Accessibility trust state and test apps; Spaces feature tested and gated independently.
- A Linux/Wayland reference environment may be used only for comparing user-level expectations; it is not a supported product target unless separately approved.

Use physical or properly provisioned virtual machines for integration tests. Do not infer macOS AX behavior from cross-compiling on Windows/Linux. Keep sensitive user windows out of automated test captures.

## 4. Performance and soak tests

- Synthetic 10/50/100-window planner benchmark; report p50/p95/p99 for plan generation and queue latency.
- 24-hour idle soak per platform; capture CPU, memory, event volume, crash/hang and lost-subscription recovery.
- Stress event sequence: 1,000 changes in 10 seconds; bounded queue and stable final geometry required.
- Display hotplug/DPI/permission churn; app must return to a reconciled state without restart or document the recovery action.

## 5. Security/privacy checks

- Threat-model review before beta; review unsafe Rust and native FFI at each PR touching adapters.
- Fuzz config parser and selectors; sanitize titles before UI/log output.
- Verify no network traffic in MVP and inspect packaged binary dependencies/licenses.
- Check permissions are least privilege, explained, revocable, and not bypassed.
- Verify config write is atomic and permissions are user-only where supported.
- macOS signing/notarization and Windows signing/update supply chain review before public download.

## 6. CI gates

Every pull request:

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, unit/component tests.
- Build on Windows x64 and macOS x86_64 targets; target compile is not a replacement for on-device tests.
- Dependency/license audit, secret scan, unsafe-code diff review.
- Deterministic layout golden tests and config migration fixtures.

Nightly/release candidate:

- Native OS integration matrix; performance + soak; artifact signing and reproducibility metadata.
- Publish supported OS/API capability matrix and known limitations.

## 7. Release gates

### Alpha

- Feature complete P0 on one platform; all core invariants pass; safe-mode/quit works.

### Beta

- Windows and Intel macOS vertical slices; permission onboarding; 7-day issue triage; integration tests on actual OS versions.

### Stable

- No unresolved critical/high severity defects; 24-hour soak passes; permission and crash recovery verified; documented support matrix; signed artifacts; rollback/uninstall instructions; privacy statement and license notices published.

## 8. Defect severity

- **S0 Critical:** data/security breach, OS/session instability, uncontrollable repeated movement, app cannot be stopped.
- **S1 High:** commonly used windows mismanaged, permission failure, unintended hide/minimize, crash on routine use.
- **S2 Medium:** feature inconsistency or workaround exists.
- **S3 Low:** cosmetic/docs issues.

S0/S1 block release. Every geometry/focus regression must include a reproducible state fixture or documented OS trace when possible.
