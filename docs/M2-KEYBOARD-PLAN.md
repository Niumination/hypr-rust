# M2 Plan — Keyboard-first flow on self-owned fixture (macOS Tahoe)

**Status:** DRAFT — for review/approval. No implementation yet.
**Base:** M1 head `e78d104` (PR #3, pending re-review). Branch: `macos/m2-keyboard-plan`.
**Host:** owner ThinkPad X13 Yoga Gen 1, macOS Tahoe 26.7.1 (Hackintosh). Proves only this host/build.

## Objective

A small M2 that drives the M1 adapter from **explicit keyboard-first actions** on the self-owned fixture — target selection, layout mapping to native operations, and typed failure reporting. No auto-tiling, no global hotkey loop, no workspace emulation.

## Decisions (binding for acceptance)

1. **Single fixture window; multi-window tiling deferred.** M2 drives **one** self-owned fixture window. The M1 adapter resolves one main window per pid. Multi-window tiling (2+ windows) is **deferred to a later stage** — M2 does not claim tiling.
2. **Target = registered fixture only.** Target selection picks among registered self-owned fixtures (by pid/token). Never select or mutate user windows. No general window enumeration for mutation.
3. **Keyboard = explicit action, not a loop.** Keyboard input triggers one explicit action (e.g. a key chord → one move/resize/raise). **No** background loop, **no** global hotkey daemon, **no** auto-tiling. Each action is user-initiated and one-shot.
4. **`raise` only on explicit request.** No implicit raise on focus/selection.
5. **Failure = typed error + best-known state.** On failure or rollback failure, report a typed error and the best-known frame/status. Never crash, never mask a partial result.
6. **No Spaces / workspace emulation / private APIs / general-Mac claims.** Spaces stay disabled. No CGS/SkyLight. No claim of general macOS or Apple-Mac support.
7. **Windows deferred** until a Windows boot is available.
8. **Accessibility OS permission ≠ auto-tiling consent.** AX trust is a prerequisite for the adapter, not user consent to auto-tile. Auto-tiling stays off until explicit onboarding consent (separate future stage).

## Scope

- **Target selection:** choose the registered fixture (pid/token) as the action target. Read-only discovery of registered fixtures only.
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

- [ ] Explicit keyboard action on the registered fixture performs exactly one native operation (move/resize/raise) via the M1 adapter.
- [ ] Target selection is limited to registered fixtures; user windows are never mutated.
- [ ] No background loop / global hotkey / auto-tiling runs.
- [ ] `raise` fires only on an explicit action.
- [ ] Every failure path returns a typed error + best-known frame/status; no crash, no masked partial state.
- [ ] fmt/clippy/test clean (workspace + standalone macOS crate).
- [ ] Native rerun on the owner host: explicit move/resize/raise on the fixture; invalid geometry rejected; stale target typed; permission-denied marked NOT RUNTIME TESTED unless safely isolated.
- [ ] No Spaces/private-API/general-Mac claims in docs.

## Branch / workflow

1. Wait for PR #3 re-review + owner approval (M1 must merge first).
2. Implement on `macos/m2-keyboard-plan` (or a focused sub-branch) from the merged M1 head.
3. Run fmt/clippy/tests + native acceptance on the owner host.
4. Push, report exact SHAs + results, request re-review. Do not merge without approval.

## Risks / limitations

- Single display 1.0x; multi-display/multi-DPI untested.
- AX is not transactional — rollback is best-effort (M1 §7).
- Lock check is a test guard, not absolute PID-reuse proof (M1 C14).
- Permission-denied path likely NOT RUNTIME TESTED (would disrupt TCC).
- Proves only the owner's Hackintosh/Tahoe host, not general macOS.
