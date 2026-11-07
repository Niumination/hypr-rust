# Hardware / display matrix — macOS (host-scoped evidence)

**Status:** host-bound matrix, probe + unit rows only · 2026-11-07
**Scope:** what display/hardware behavior is actually verified today, and what each unverified row needs. PRD requirement ("layout correctness on reference test matrix"; "capability matrix per platform/OS build"). Windows rows live in [CAPABILITY-MATRIX](./CAPABILITY-MATRIX.md).

## Environment probed (read-only, 2026-11-07)

`system_profiler SPDisplaysDataType` on the owner's host:

- GPU: Intel UHD Graphics 620 (built-in)
- **1 display, 1920 × 1080, "UI Looks like" 1920 × 1080 → scale 1.0×**, Main Display: Yes, Mirror: Off, Online: Yes
- Host: Lenovo ThinkPad X13 Yoga Gen 1 (Intel i5-10310U), macOS Tahoe 26.7.1 build 25G241 — **not Apple hardware** (see [USER-DEVICE-PROFILE](./USER-DEVICE-PROFILE.md))

No display topology was changed for this probe; nothing was moved, resized or hidden.

## Matrix

| # | Scenario | State | Evidence / what it needs |
|---|---|---|---|
| 1 | Single display 1920×1080 @1.0×, main-display work area = bounds − 30 pt menu-bar strip | **PASS (host + unit)** | Probe above; `work_area_from` unit tests (`display_geometry_tests`); coordinate-convention record in PLATFORM-SPIKES (menu-bar clamp y 12→30 observed live) |
| 2 | Fullscreen window excluded from targets (heuristic: covers work area **and** starts above it) | **PASS (unit)** + host-corroborated | `fullscreen_heuristic_separates_fullscreen_from_zoomed` (4 shapes incl. zoomed/ordinary/partial); host: `skipped (minimized/fullscreen/off-display)` on **351** status lines in `~/.hypr-rust/shell.log` (first at line 622) |
| 3 | Minimized window excluded from targets | **PASS (unit)** + host-corroborated | `drop_non_target_windows_removes_non_normal_and_off_display`; same 351 log lines (the status line merges minimized/fullscreen/off-display — individual attribution is not claimed) |
| 4 | Off-display window excluded (window center outside the display rectangle) | **PASS (unit)** + host-corroborated | `center_inside_membership_is_boundary_exact` (half-open edges, second-display rectangle, mostly-off-screen-but-centered-in); same 351 log lines |
| 5 | Degenerate display bounds never produce a non-positive work area | **PASS (unit)** | `work_area_of_a_degenerate_display_stays_positive` |
| 6 | Display work-area/resolution change **while running** (re-read on next reflow) | **PARTIAL (code-read only, NOT runtime-tested)** | `work_area()` re-reads `CGDisplayBounds` on every reflow call — no cached value — but no live resolution/scale change was performed on the host. Testing it needs: owner changes resolution/scale mid-session, then triggers a reflow |
| 7 | Second display (extended desktop) | **NOT RUN** | Needs: a second display attached to the host. Code posture: `center_inside` membership and `work_area_from` are origin-aware (unit-tested with a right-hand display rectangle), but the shell always queries `CGMainDisplayID()` only — windows on other displays are classified off-display and left untouched. No multi-display runtime claim exists |
| 8 | Mixed-DPI / scaled display (e.g. 2× "Looks like" resolution) | **NOT RUN** | Needs: a display whose UI scale ≠ 1.0×. The adapter documents `scale_factor: 1.0` and `bounds == work_area` as a POC limit (PLATFORM-SPIKES); HiDPI point/pixel behavior is unverified |
| 9 | Apple-branded Mac (any model) | **NOT RUN** | Needs: Apple hardware. All rows above are host-bound to a ThinkPad hackintosh |
| 10 | macOS versions other than Tahoe 26.7.1 (PRD floor: Ventura 13) | **NOT RUN** | Needs: those OS builds. Current builds pin `MACOSX_DEPLOYMENT_TARGET=26.0` — the Ventura 13 floor is not met by today's build and not claimed |
| 11 | Display hot-plug / sleep-wake topology change | **NOT RUN** | Needs: owner attaches/detaches a display or sleeps/wakes the host, then reflows. No `CGDisplayRegisterReconfiguration` observer exists; behavior outside a reflow is undefined by design (nothing is subscribed) |

## Rules honored by this matrix

- Every row above is read-only probing or unit tests; **no window was moved, resized, minimized or hidden** to produce evidence (AGENTS.md: never emulate workspaces by hiding/minimizing; PLATFORM-SPIKES evidence rules).
- "PASS (unit)" means the logic is tested without a display; it is **not** runtime validation on that hardware (AGENTS.md §7).
- "NOT RUN" rows state the exact missing environment — a successful compile never upgrades them.
- Row 2–4 log counts are from the owner's live auto-tiling sessions; the merged status line means per-cause counts are **not** claimed.

## Related

- [CAPABILITY-MATRIX](./CAPABILITY-MATRIX.md) — per-capability product states (this doc is the display/hardware slice).
- [PLATFORM-SPIKES](./PLATFORM-SPIKES.md) — spike environment record, coordinate convention, single-display POC limits.
- [ROADMAP](./ROADMAP.md) item "Hardware/display matrix" — Apple-Mac/multi-display repeat still pending hardware.
