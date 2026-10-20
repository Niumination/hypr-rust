# M2 — Keyboard-first flow on self-owned fixture (macOS Tahoe)

**Status:** IMPLEMENTED (2026-10-19). Awaiting re-review.
**Base:** M1 head `e78d104` (PR #3, merged). Branch: `macos/m2-keyboard-plan`.
**Host:** owner ThinkPad X13 Yoga Gen 1, macOS Tahoe 26.7.1 (Hackintosh). Proves only this host/build.
**Implementation:** `crates/platform-macos/src/bin/ax-keyboard.rs` — explicit one-shot actions (`select` / `move` / `resize` / `raise`) on the registered fixture, gated by the shared fixture verification (`src/fixture.rs`). No loop, no global hotkey, no auto-tiling.

## Objective

A small M2 that drives the M1 adapter from **explicit keyboard-first actions** on the self-owned fixture — target selection, layout mapping to native operations, and typed failure reporting. No auto-tiling, no global hotkey loop, no workspace emulation.

## Decisions (binding for acceptance)

1. **Single fixture window; multi-window tiling deferred.** M2 drives **one** self-owned fixture window. The M1 adapter resolves one main window per pid. Multi-window tiling (2+ windows) is **deferred to a later stage** — M2 does not claim tiling.
2. **Target = the single registered fixture only.** Select/validate the one currently-registered fixture by PID/token. Never select or mutate user windows. No general window enumeration for mutation. Multi-fixture / registry support is **deferred**.
3. **Keyboard = explicit action via focused UI/CLI, not a loop.** Keyboard input is received through the focused UI/CLI and triggers one explicit action (e.g. a key chord → one move/resize/raise). **No** background loop, **no** global hotkey daemon, **no** auto-tiling. Each action is user-initiated and one-shot.
4. **`raise` only on explicit request.** No implicit raise on focus/selection.
5. **Failure = typed error + best-known state.** On failure or rollback failure, report a typed error and the best-known frame/status. Never crash, never mask a partial result.
6. **No Spaces / workspace emulation / private APIs / general-Mac claims.** Spaces stay disabled. No CGS/SkyLight. No claim of general macOS or Apple-Mac support.
7. **Windows deferred** until a Windows boot is available.
8. **Accessibility OS permission ≠ auto-tiling consent.** AX trust is a prerequisite for the adapter, not user consent to auto-tile. Auto-tiling stays off until explicit onboarding consent (separate future stage).

## Scope

- **Target selection:** select/validate the single currently-registered fixture (pid/token) as the action target. Read-only; no enumeration of user windows.
- **Layout mapping:** map a small set of explicit keyboard actions to native operations via the M1 adapter:
  - move to a position (e.g. arrow/ctrl+arrow → `set_window_frame` position),
  - resize to a size (e.g. a chord → `set_window_frame` size),
  - raise (explicit chord → `raise`).
- **Failure reporting:** surface `AxError` variants (PermissionDenied / StaleTarget / InvalidGeometry / *WriteFailed / VerificationFailed) as typed, human-readable results with best-known frame.
- **Read-back verification:** reuse the M1 `set_window_frame` verified-frame contract.

## Out of scope

- Auto-tiling (any automatic arrangement).
- Multi-window tiling / layout of 2+ windows.
- Global hotkey daemon / background loop / startup service.
- Spaces / virtual-desktop switching or emulation.
- Windows.
- Onboarding/consent flow for auto-tiling.

## Acceptance criteria

- [x] Explicit action on the registered fixture performs exactly one native operation (move/resize/raise) via the M1 adapter (`ax-keyboard move/resize/raise`).
- [x] Target selection is limited to the single registered fixture (`ax-keyboard select`); user windows are never mutated.
- [x] No background loop / global hotkey / auto-tiling — each action is an explicit one-shot CLI invocation.
- [x] `raise` fires only on an explicit action (`ax-keyboard raise`).
- [x] Every failure path returns a typed error + best-known frame/status (AxError variants, FixtureError); no crash, no masked partial state.
- [x] fmt/clippy/test clean: workspace 6/6, standalone macOS crate 15/15, fmt/clippy clean.
- [x] Native rerun on the owner host (2026-10-19): select ok · no-token move → exit 5 · move ok · resize ok · raise ok · invalid resize (0 h) → exit 1, window unchanged · stale → exit 4.
- [x] No Spaces/private-API/general-Mac claims in docs.

## Native rerun (verbatim, 2026-10-19, token redacted)

```
$ ax-keyboard select --target <pid> --token <hex>
ok fixture pid=<pid> frame={x: 100, y: 668, width: 420, height: 312}
$ ax-keyboard move <pid> 120 120                      # no token
error: unregistered target (fixture identity verification failed)
no-token-exit=5
$ ax-keyboard move --target <pid> --token <hex> 120 120
ok moved {x: 120, y: 120, width: 420, height: 312}
$ ax-keyboard resize --target <pid> --token <hex> 480 320
ok resized {x: 120, y: 120, width: 480, height: 320}
$ ax-keyboard raise --target <pid> --token <hex>
ok raised
$ ax-keyboard resize --target <pid> --token <hex> 0 320
error: invalid geometry (non-finite or non-positive size)
invalid-exit=1
$ kill <pid>; sleep 1
$ ax-keyboard select --target <pid> --token <hex>
error: stale target (process/window gone)
stale-exit=4
```

## Verification summary

- `ax-keyboard` reuses the M1 fixture gate (`src/fixture.rs`): token + flock lock + pid alive, fail closed. Only `EWOULDBLOCK`/`EAGAIN` counts as "lock held".
- `move`/`resize` read the current frame, then apply an explicit `set_window_frame` (verified, rollback on failure). `raise` is explicit only.
- Fixture verification extracted to `src/fixture.rs` and shared by `ax-probe` (M1) and `ax-keyboard` (M2) — no duplicated gate logic.
- Every failure path reports a typed error with rollback status and best-known frame via `format_ax_error` for ALL error variants: transaction variants carry their own `actual` frame + `rollback_restored`; non-transaction variants report `rollback_restored=n/a` + the caller-supplied frame (`<unavailable>` when none was read). `move`/`resize` pass the pre-read frame so `InvalidGeometry` reports it. Unit-tested for all variants.
- Permission-denied path: NOT RUNTIME TESTED (would disrupt TCC).

## Branch / workflow

1. ~~Wait for PR #3 re-review + owner approval~~ — done: PR #3 merged to main (`07f6059`).
2. Implement on `macos/m2-keyboard-plan` from the merged M1 head — done: `ax-keyboard.rs` + shared `fixture.rs`.
3. Run fmt/clippy/tests + native acceptance on the owner host — done: 21/21, native rerun green.
4. Push, report exact SHAs + results, request re-review. Do not merge without approval.

## Risks / limitations

- Single display 1.0x; multi-display/multi-DPI untested.
- AX is not transactional — rollback is best-effort (M1 §7).
- Lock check is a test guard, not absolute PID-reuse proof (M1 C14).
- Permission-denied path likely NOT RUNTIME TESTED (would disrupt TCC).
- Proves only the owner's Hackintosh/Tahoe host, not general macOS.
