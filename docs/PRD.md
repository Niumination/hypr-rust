# Product Requirements Document

**Product:** Hypr-Rust (working title)  
**Status:** Interview baseline — draft for feasibility validation  
**Last updated:** 2026-10-17  
**Owners to assign:** Product, Rust lead, Windows platform lead, macOS platform lead, QA/security

## 1. Problem statement

Users want a keyboard-centric dynamic tiling workflow inspired by Hyprland while using Windows 11 or an Intel Mac. Hyprland itself is a Linux/Wayland compositor and is not being ported as a compositor. This product is an original native desktop window-management application that manages existing windows through supported OS APIs. It must communicate platform limitations clearly and must not imply affiliation with or exact compatibility with Hyprland.

## 2. Interview decisions — baseline

| Area | Decision | Status / caveat |
|---|---|---|
| Product boundary | Native tiling window manager/controller, not a compositor port | Agreed |
| Targets | Windows 11 x64 + Intel macOS 13 (Ventura) or later | Agreed; verify Intel hardware and API behavior on exact builds |
| Development strategy | Build/test both platform vertical slices in parallel, with a shared Rust core | Agreed; feasibility spikes first |
| App shell | Tray app on Windows; menu-bar app on macOS; settings UI | Agreed |
| Tiling | BSP, master/stack, monocle, and columns/rows; configurable gaps/margins/ratio | Agreed as initial layout set |
| Keyboard | Keyboard-first; workspace shortcuts are a user priority; use OS-appropriate defaults that users can change | Agreed; actual workspace controls gated by platform API validation |
| Workspace semantics | Native Windows Virtual Desktop / macOS Spaces only; no app-owned logical workspace and no minimize/hide fallback | Agreed; optional per platform, only enabled after proof of public-API support |
| Unsupported workspace operation | Leave windows unchanged and report; do not force hide/minimize | Agreed |
| Configuration | Settings UI; versioned config may be internal/exportable, but no Hyprland syntax promise | Agreed |
| Auto-tiling | Offered during onboarding and begins only after explicit user confirmation | Agreed |
| Window rules | User-configurable for ordinary app windows; mandatory safety exclusions remain non-overridable | Agreed |
| Quit behavior | Leave windows at their last position; do not restore initial geometry | Agreed |
| Distribution | Direct download/installer first; code signing/notarization required for release | Agreed in principle |
| Use cases | Developer workflow and general desktop use | Agreed; concrete representative apps to be selected for test matrix |

## 3. Target users

- Developers and power users who prefer keyboard-driven placement.
- Users who want a similar workflow on Windows 11 and Intel macOS.
- Users who expect transparent permissions, a visible status, and a reliable pause/quit control.

## 4. Product principles

- Native OS behavior first; shared semantics second.
- Predictable layouts, typed capability reporting, reversible/interruptible actions.
- Use public APIs; no private OS APIs in MVP.
- Explain permissions and ask before starting automatic window management.
- Keep an emergency pause and clean quit available independently of user rules.
- Never promise compositor-level rendering or cross-platform feature parity without test evidence.
- No telemetry or network dependency for core operation.

## 5. Scope

### P0 — cross-platform core

- Native application lifecycle: Windows tray, macOS menu bar, settings UI, pause, quit, status and diagnostics.
- Window/display discovery and capability reporting where supported.
- Shared deterministic Rust layout engine: BSP, master/stack, monocle, columns/rows; gaps, margins, master ratio, min-size policy and floating rules.
- Reflow when eligible windows/display state changes; debounce event storms and suppress feedback from app-issued operations.
- Keyboard shortcuts: focus navigation, reflow, layout select, ratio adjustment, floating toggle and enable/pause management. Platform-appropriate defaults; user-editable; detect/report registration conflicts.
- User-defined ordinary-window rules (manage/float/ignore) and window selectors where metadata is available.
- Settings UI with validation, safe defaults and capability/permission information.
- Auto-tiling offered at onboarding; do not tile until the user explicitly enables it.
- Mandatory exclusions: app's own windows, system/desktop windows, and targets/operations the OS reports as unsupported or inaccessible. These cannot be overridden by a user rule.
- Per-operation apply results; denied or unsupported operations are visible and do not cause retry loops.
- Direct Windows installer and macOS app/DMG packaging for beta/release; signing/notarization plan must be proven.

### Conditional feature — native workspace shortcuts

Workspace is specifically the OS-native Windows Virtual Desktop or macOS Space, not an application-owned grouping. Implement only after a feasibility spike demonstrates required operations using public APIs on the declared OS versions. Required support contract: detect/identify the active desktop/space, switch to a selected desktop/space, move the focused window to a selected desktop/space, observe changes, and report denied operations. Enable workspace shortcuts on a platform only if this complete set is validated there.

- Feature is enabled per platform only if the required behavior is verified and release-tested.
- If not supported or not yet verified, disable workspace actions on that platform and explain why; other tiling features remain available.
- Never simulate workspace switching by minimizing, hiding, or forcefully changing visibility of windows.
- If an individual operation fails, leave that window unchanged and report the failure; do not claim workspace parity.
- At startup, follow the OS desktop/space already active. Do not force a desktop switch.
- Workspace capability is not a release blocker for the core tiling manager.

### Not in scope

- Replacing Windows Explorer, macOS WindowServer, a login/session manager or Wayland display server.
- App-managed logical workspaces, automatic minimize/hide fallback, or guaranteed hide/show of other apps.
- Compositor effects such as guaranteed blur, borders behind arbitrary windows, or pixel-identical rendering.
- Screen recording, input injection, arbitrary app introspection, remote control, telemetry/network service.
- Plugin ABI, arbitrary shell command execution in config, exact Hyprland config compatibility.

## 6. User stories and acceptance criteria

### Tiling and reflow

**As a user**, when I enable auto-tiling, eligible windows should be arranged by my selected layout.

- Auto-tiling remains off until explicitly enabled during onboarding/settings.
- Two eligible windows in BSP produce non-overlapping rectangles within the usable display area, within 1 DIP/pixel rounding tolerance.
- Repeated application of an identical plan is idempotent.
- Unsupported/denied operations produce a per-window result and do not crash, spin or block all other windows.
- User can pause management immediately; quitting leaves windows at their last position.

### Keyboard and settings

- Default shortcut presets are platform appropriate, editable, and conflicts/reserved combinations are surfaced.
- Shortcut registration failure leaves tray/menu UI usable and is visible in diagnostics.
- Layout, ratio, gaps, ordinary app rules, auto-tiling and capability state are configurable in the settings UI.
- Emergency pause and quit are independent of user-defined rules and config validity.

### Native workspace (conditional)

- Workspace actions are exposed only where platform capability tests pass for the exact supported OS build.
- No code path minimizes or hides windows as a workspace substitute.
- If a workspace action is unavailable or denied, affected windows remain unchanged, the user gets a clear per-operation message, and there is no unbounded retry.
- Startup follows the currently active OS desktop/space; app does not force a switch.
- If capability requirements fail on a platform, workspace shortcuts are disabled there and this does not block release of core tiling features.

### Permissions and safety

- macOS explains Accessibility access before opening System Settings; app separately reports observe/control capability and never reports a denied operation as success.
- Windows does not request administrator privileges by default; cross-integrity/elevated target limitations are explained.
- Mandatory system/app-self/unsupported exclusions cannot be overridden.
- The app does not attempt geometry restoration on quit; it simply stops managing and leaves last positions.

## 7. Functional requirements

- FR-01 Domain uses stable runtime window keys; native handles/AX references are never persisted.
- FR-02 Layout calculation is deterministic for identical state/config.
- FR-03 Planner emits an immutable plan; platform executor returns per-operation result and actual observed state.
- FR-04 Events normalize add/remove/geometry/focus/display/permission/workspace changes where observable.
- FR-05 Own writes are correlated; feedback loops are suppressed.
- FR-06 Settings schema is versioned and safely migrated.
- FR-07 Mutating actions support dry-run/preview where practical; emergency pause is always available.
- FR-08 Routine logging omits window titles by default; diagnostic export is explicit and redacted.
- FR-09 Auto-tiling may start only after explicit opt-in; permission grant alone is not opt-in to auto-management.
- FR-10 Workspace capabilities are advertised per OS/build; unsupported is a first-class state.
- FR-11 Workspace must never call minimize/hide as a fallback for desktop/space movement.
- FR-12 Shutdown cancels observers/hotkeys and leaves all windows in their latest OS-accepted geometry.

## 8. Non-functional requirements (initial targets; validate in prototype)

- Startup to ready: p95 < 2 seconds, excluding first-time permission prompt.
- Event-to-plan latency: p95 < 100 ms for 50 managed windows; OS operation completion measured separately.
- Idle CPU target: <1% average over 5 minutes on reference hardware; event-driven observation preferred.
- Memory target: <100 MiB resident for MVP, excluding platform libraries.
- No network required; no telemetry by default.
- Crash-free sessions >99.5% during a 24-hour soak per platform before stable release.

## 9. Compatibility and release gates

- Windows: Windows 11 x64; test multi-monitor/mixed DPI; document virtual desktop features only where explicitly verified.
- macOS: Intel x86_64, Ventura 13+; validate Accessibility, distribution identity, multi-display geometry and app-specific support on real Intel hardware.
- Direct distribution requires signing/notarization and tested install/update/uninstall path.
- Publish a capability matrix per platform/OS build. Do not claim support for apps/windows that fail capability checks.

## 10. Product metrics

- Layout correctness on reference test matrix.
- Unsupported-operation rate by platform/API reason in local logs.
- Onboarding opt-in and safe-pause usability (no telemetry by default; user research/opt-in diagnostics only).
- Workspace capability pass/fail per declared OS build.
- User-reported accidental focus, geometry, permission or workspace issues.

## 11. Remaining open questions

1. Exact Windows 11 builds and representative hardware/app list for acceptance tests.
2. Specific applications (3–5) to anchor developer and general-use validation.
3. Exact Windows 11 builds and representative apps for the native workspace test matrix; required operations are fixed in ADR-0003.
4. UI framework/native shell and shortcut presets, selected after platform spikes.
5. Final legal review of product name, Hyprland references and distribution notices.
