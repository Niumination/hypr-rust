# ADR-0005: Batch Multi-Window Application Accepted With Best-Effort Rollback

- **Status:** Accepted (owner decision: change the semantics; supersedes ADR-0004)
- **Date:** 2026-10-22
- **Decision owners:** Product + owner
- **Supersedes:** [ADR-0004](./0004-batch-multiwindow-deferred-no-partial.md)

## Context

ADR-0004 deferred native batch multi-window application because no reviewed public macOS API provides all-or-nothing semantics for a multi-window geometry batch, and best-effort rollback was not accepted as meeting the strict no-partial requirement. The pure planner (`layout-core::domain`) is OS-neutral and complete; the blocker was the native executor.

The public Accessibility API exposes only per-element writes (`AXUIElementSetAttributeValue`), each returning its own error. There is no public cross-element transaction and no public reservation that would make a batch atomic. Private WindowServer APIs (SkyLight/CGS) are out of scope.

## Decision

1. The owner changes the semantics: a native batch multi-window operation is **accepted with best-effort rollback**. It is no longer required to be atomic.
2. A batch applies a verified frame to each target in order. On the first failure the batch stops and restores every already-changed window to its pre-batch frame, verifying each restore.
3. A failed batch that could not restore everything is reported honestly: the actual observed frames are returned and `rollback_all_restored == false`. **No claim of atomicity is made**, and a partial state is never hidden.
4. Master selection stays as recorded in ADR-0004 §5: the window focused at the time of the explicit command is the master; the planner never raises or focuses a window.
5. Read-only research, the pure planner, and the single-window M1/M2 paths remain unchanged.

## Consequences

### Positive

- Multi-window tiling can be delivered without an OS atomic batch primitive.
- Failure reporting is honest: the caller learns the real state, not a guess.

### Negative / accepted limitations

- A failed batch may leave a partial state; the report states this and gives the observed frames.
- Best-effort rollback is not a substitute for atomicity and must not be described as such.
- Native evidence is host/build-bound until it is run on the target.

## Alternatives rejected

- **Requiring a public all-or-nothing API:** no such API exists on the target; keeping the strict requirement blocks delivery indefinitely (ADR-0004).
- **Private/undocumented APIs (SkyLight/CGS) for atomic batching:** rejected — public API only.

## Addendum — owner acceptance of the residual risk (2026-10-24)

The owner's standing acceptance for any multi-window mutation remains **strict no-partial**; this ADR's best-effort semantics do not satisfy that acceptance. After the host evidence in [`ARENA-VERIFICATION.md`](../ARENA-VERIFICATION.md) Paths F–H (a mid-batch write rejection produces a genuine cross-window partial that only per-window rollback can chase, and AX offers no prepare/commit transaction that could remove the residual race), the owner explicitly chose the **status quo**: the fixture-bound shell `Reflow:` path keeps these semantics unchanged. Binding conditions of that decision: the path is never advertised or documented as no-partial-compliant, every report labels batch outcomes as best-effort, the reserved-fixture boundary stays the only mutation surface, and any future productization (non-fixture targets, scheduled/auto application) requires a new decision. A preflight protocol (write-capability probes plus read-back) was evaluated and can only narrow the failure classes, not guarantee the constraint; the evidence label for any fixture rollback runs remains "upstream best-effort demonstrated", not acceptance.

## References

- Superseded: [ADR-0004](./0004-batch-multiwindow-deferred-no-partial.md)
- M1 report: `docs/reports/MACOS-M1-AX-ADAPTER-2026-10-18.md`
- Arena brief: `docs/ARENA-VERIFICATION.md`
- Apple docs: `AXUIElementSetAttributeValue`, `AXUIElementIsAttributeSettable`
