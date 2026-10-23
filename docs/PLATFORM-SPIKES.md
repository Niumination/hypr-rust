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

### Spike 1 addendum — batch multi-window application (macOS, recorded result)

Implemented with best-effort rollback ([ADR-0005](./adr/0005-batch-multiwindow-best-effort-rollback.md)), explicitly **not atomic**. Recorded on the owner's ThinkPad X13 Yoga Gen 1, macOS Tahoe 26.7.1 build 25G241, AX granted; batch code and CLI from branch `work/phase1-domain` commit `085f5cb`; runs on 2026-10-22 (single-fixture) and 2026-10-23 (two-window + denied path). Targets are self-owned `HYPR-SPIKE-TARGET` windows only; no titles read.

| Case | Command | Result |
|---|---|---|
| CLI gates (no AX call when they fail) | `ax-probe batch` variants | no args → usage, exit 1; no/bad fixture registration → exit 5; plan must include the registered fixture and be a regular non-symlink owner-only (0600) file |
| Two-window success | `batch --target <A> --token <hex> --plan [A, B]` | both frames applied and verified; `rollback_all_restored=true`, exit 0; per-window `frame` read-backs matched |
| Two-window rollback | plan `[A→new frame, B→new frame, <live pid with no window>]` | third target failed (`-25204 cannotComplete`) after A and B were applied; batch stopped and restored A and B, both verified by read-back; exit 1 |
| Permission-denied | read-only `ax-probe frame 1` in an AX-untrusted launchd user-domain job | `error: permission denied (Accessibility not granted)`, exit 3; no AX call made; one context only (see `ARENA-VERIFICATION.md` §6) |

**Exact host/build:** as in Spike 0 record; AX granted to the controlling terminal app. **Evidence & caveats:** `docs/ARENA-VERIFICATION.md` §6 — executed on the owner's host, not independently run by arena.ai; second window B came from a locally compiled test-only unregistered spawner variant (not committed, not part of PR #1).

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
