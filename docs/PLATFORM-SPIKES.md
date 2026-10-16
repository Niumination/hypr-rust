# Native Platform Spike Plan & Evidence Template

**Goal:** determine whether core window control and native workspace switch/move can be implemented safely on the declared OS builds using public APIs. This document is a test protocol, not a statement that the APIs have passed.

## Spike 0 — environment record

Complete before testing:

- Date / tester / source branch + commit:
- Machine model and CPU:
- OS edition, exact version/build:
- Display topology, resolution and scale/DPI:
- App build and signing identity:
- Permission state (especially macOS Accessibility):
- Test applications and versions:
- Relevant API/documentation links:

Do not attach private window titles, screenshots or personal data unless explicitly required and redacted.

## Spike 1 — ordinary window management (release prerequisite)

For each representative app, test discovery, eligibility, current frame, move, resize, focus request, event observation, close/stale handle, denied operation and pause/quit. Include normal, dialog/modal, fullscreen, minimized, maximized, elevated (Windows), permission-revoked/inaccessible (macOS), mixed-DPI/multi-display cases.

**Pass criteria:** ordinary supported windows can be discovered and moved/resized repeatedly without crash, focus theft beyond the requested action, unbounded retry or geometry feedback loop. Unsupported windows are reported and excluded safely.

## Spike 2 — native workspace contract (conditional feature gate)

Required set—test each operation independently, then as an end-to-end workflow:

1. Detect and identify the active native desktop/space and a target desktop/space.
2. Switch to the selected target desktop/space.
3. Move the currently focused window to a selected desktop/space.
4. Observe user-initiated changes and report denials/stale targets without hiding or minimizing windows.
5. Repeat with signed app identity, permissions, multiple displays, app restart and common target applications.

**Pass:** every operation is repeatable on the exact supported OS build using reviewed public APIs, with expected user-visible result and no private APIs or hide/minimize workaround.

**Fail/partial:** mark workspace unsupported on that OS/build, record which operation failed and why, and continue core tiling work. Do not ship partial workspace shortcuts under a full-support label.

### Windows result

| Operation | Pass/fail/not available | Evidence (API docs, test case, logs) | Native error / limitation | Reviewer |
|---|---|---|---|---|
| Detect active/target desktop | | | | |
| Switch desktop | | | | |
| Move focused window | | | | |
| Observe changes/report denial | | | | |

**Decision:** `SUPPORTED` / `DISABLED`  
**Exact OS build and tested app list:**  
**Known limitations:**

### macOS Intel result

| Operation | Pass/fail/not available | Evidence (API docs, test case, logs) | Native error / limitation | Reviewer |
|---|---|---|---|---|
| Detect active/target Space | | | | |
| Switch Space | | | | |
| Move focused window | | | | |
| Observe changes/report denial | | | | |

**Decision:** `SUPPORTED` / `DISABLED`  
**Exact macOS version/build, Intel model and signing identity:**  
**Permission state and known limitations:**

## Evidence and handoff rules

- Prefer concise text logs, API documentation links and reproducible steps. Redact app titles/user data.
- Record successful and failed cases; a missing event/result is not proof of unsupported behavior until checked against API docs and controls.
- Include the exact commit tested and test commands. The receiving reviewer must be able to reproduce the result.
- Commit spike findings to the upstream repo in a dedicated PR; do not put unverified conclusions into user-facing claims.
