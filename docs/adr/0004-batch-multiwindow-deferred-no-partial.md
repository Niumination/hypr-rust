# ADR-0004: Batch Multi-Window Application Deferred Under Strict No-Partial Requirement

- **Status:** Accepted (owner decision: retain the strict all-or-nothing requirement)
- **Date:** 2026-10-22
- **Decision owners:** Product + owner

## Context

M1 (`set_window_frame`) and M2 (`ax-keyboard` `select`/`move`/`resize`/`raise`) are native-verified on the owner's reported host (Lenovo ThinkPad X13 Yoga Gen 1, Intel i5-10310U, macOS Tahoe 26.7.1 build 25G241). Both operate on a **single self-owned fixture window at a time**.

A read-only capability probe (2026-10-21, same host) confirmed that the public Accessibility API exposes per-application window enumeration (`AXWindows`), per-element PID identity (`AXUIElementGetPid`), and readable `AXPosition`/`AXSize` for 2–3 self-owned fixture windows. The probe made **no AX writes** and proved nothing about batch atomicity.

The reviewed Apple Accessibility API does not provide a general cross-element transaction. `AXUIElementIsAttributeSettable` is a capability query, not a reservation or a guarantee; `AXUIElementSetAttributeValue` returns per-element errors. M1's rollback (`set_window_frame_txn`, C9) is **best-effort**: it attempts to restore state and reports failure, but a failed recovery can leave partial changes.

The owner requires that a batch operation must not leave partial changes.

## Decision

1. Native **batch multi-window application is DEFERRED / BLOCKED** by the strict no-partial requirement. It is **not** declared permanently unsupported; reopening requires evidence or an explicit owner decision (see "Reopen conditions").
2. **Pure layout planning may continue.** `layout-core::compute_layout` with `LayoutKind::MasterStack` and its tests have no OS side effects and remain in scope. The planner is not the blocker.
3. **Read-only research may continue** (window enumeration, geometry reads, settable capability queries) under the existing fixture-only, no-mutation rules.
4. **No AX writes, no apply+rollback executor, no multi-window layout executor, and no UI/hotkey/auto-tiling/workspace changes** are authorized while this ADR stands.
5. **Master selection (recorded for when the gate reopens):** the window focused at the time of the explicit command becomes the master; other windows follow a stable order. No implicit raise/focus.

## Reopen conditions

Batch multi-window application may be unblocked only by one of:

- A reviewed **public** API mechanism that demonstrably provides all-or-nothing semantics for a multi-window batch on the target OS/build, with native evidence; or
- An **explicit owner decision** to change the semantics. Accepting documented best-effort rollback (stop-on-first-failure, roll back all changed windows, verify, and report the actual partial state honestly) would be such a decision. **Best-effort rollback alone is not accepted as meeting the strict requirement.**

## Risks and accepted limitations

- Best-effort rollback cannot guarantee the absence of partial changes on failure. Honest reporting of a rollback failure does not remove the partial state.
- Path A success on normal operations does **not** prove the all-or-nothing guarantee under failure. M1/M2 must not be described as meeting it.
- Error-code mapping recorded for the probes: `-25208` = `AXError.notImplemented`; `-25204` = `AXError.cannotComplete`.
- `AXSize settable=false` was observed on the **probe fixture**; the M2 `resize` PASS came from a **different fixture** (the registered M2 path). No write was attempted on the probe element. Cross-fixture comparison proves nothing about the same element.
- Fixture-only evidence on one host/build does not generalize to other applications, hosts, or macOS versions.

## Consequences

### Positive

- No misleading "atomic batch" claim is made.
- The OS-independent planner and its tests can proceed without waiting for the native gate.
- The deferral is reopenable with public evidence or an explicit decision.

### Negative / accepted limitations

- Batch multi-window tiling is not delivered while this ADR stands; users manage multiple windows manually.
- Cross-platform parity is not claimed.

## Alternatives rejected

- **Best-effort rollback as a substitute for the strict requirement:** rejected. Reporting failure honestly does not make a partial change disappear.
- **Private/undocumented APIs (e.g., SkyLight/CGS) to obtain atomic batching:** rejected — public API only.

## References

- M1 report: `docs/reports/MACOS-M1-AX-ADAPTER-2026-10-18.md`
- M2 plan: `docs/M2-KEYBOARD-PLAN.md`
- Arena brief: `docs/ARENA-VERIFICATION.md`
- Apple docs: `kAXWindowsAttribute`, `AXUIElementIsAttributeSettable`, `AXUIElementSetAttributeValue`
