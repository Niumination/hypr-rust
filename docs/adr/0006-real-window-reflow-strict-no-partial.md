# ADR-0006: Real-Window Reflow Scope (All Normal Windows) With Strict No-Partial Application

- **Status:** Accepted (owner decision; fulfils the ADR-0005 addendum's "productization needs a new decision")
- **Date:** 2026-10-27
- **Decision owners:** Product + owner
- **Relates to:** [ADR-0005](./0005-batch-multiwindow-best-effort-rollback.md) (fixture-bound status quo), [ADR-0004](./0004-batch-multiwindow-deferred-no-partial.md) (superseded), [ADR-0001](./0001-product-boundary.md)

## Context

ADR-0005's addendum (2026-10-24) accepted best-effort rollback **status quo for the
fixture-bound `Reflow:` path only** and bound any future productization — non-fixture
targets or scheduled/auto application — to a new owner decision. The owner's standing
acceptance for multi-window mutation outside that status quo remains **strict
no-partial**.

The first-run loop closed on 2026-10-26/27 (per-field panel validation, in-panel save
status, zero-rule body fix, JSON escape completeness, AX grant under the bundle
identity), all evidence fixture-bound. The owner then asked to move to real app
windows and chose, explicitly:

1. **Scope (a): all normal windows** — an explicit reflow command targets the normal
   windows of every regular app, not only the fixture; rules may protect individual
   apps; mandatory exclusions stay non-overridable.
2. **Strict no-partial** for this path — one failure means every changed window returns
   to its original position; the operation refuses more eagerly rather than accepting a
   partial outcome.

The public Accessibility API still provides no cross-element transaction
([ADR-0005](./0005-batch-multiwindow-best-effort-rollback.md) context; Paths F–H showed
a genuine mid-batch partial that only per-window rollback can chase). This ADR
therefore defines strict no-partial **operationally** — what the product promises and
how it behaves — rather than claiming an atomicity the platform cannot provide.

## Decision

### 1. Scope: the explicit reflow command becomes non-fixture

- The menu `Reflow:` commands target every **normal window** (`Application` role) of
  every regular app on the main display, enumerated at command time.
- Auto-tiling stays **off**: nothing is applied unless the user picks a reflow command.
  Quit terminates the app and leaves windows at their last OS-accepted position.
- **Mandatory exclusions are unchanged and cannot be overridden by rules**: this
  controller's own windows, system UI, and desktop surfaces are always excluded
  (planner gates). Dialog inclusion keeps following `include_dialogs` plus per-app
  rules.
- **Rules gain app protection**: a matching `exclude` rule now excludes that app's
  normal windows from tiling as well as its dialogs (before this ADR, rules could only
  shift dialog policy). This is the mechanism that lets the owner protect an app.

### 2. Strict no-partial, defined operationally

For the real-window path a batch is **successful only if every target window was
written and verified**. Otherwise:

1. **Preflight before the first write**: enumerate the target apps, read every target
   frame, and build the full plan before any mutation. Any enumeration or read failure
   refuses the operation with **zero writes**.
2. **Per-window verified writes** (the existing single-window no-partial contract:
   geometry preflight, save, position+size write, read-back compare, self-restore on
   any failure), in plan order; the first failure stops the batch.
3. **Full rollback on failure**: every window already changed is restored to its
   pre-write frame and each restore is verified.
4. **Honest three-way reporting**:
   - all targets applied and verified → success;
   - failure with every restore verified → **clean refusal** ("refused, all windows
     restored") — the operation left no change;
   - failure with any unverifiable restore → **loud partial-state status** naming the
     observed frames; never hidden, never reported as success.

The preflight + restore protocol narrows failure classes but cannot eliminate the
residual race documented in Paths F–H; the promise is therefore *no partial is ever
reported as success and every detectable failure refuses before writing* — not
platform-level atomicity. Status/label text must never use the word "atomic".

### 3. Boundary after this ADR

- **Non-fixture surface:** the interactive menu reflow commands, owner-driven, one
  command per mutation.
- **Fixture-bound surface (unchanged):** all automated evidence — `--self-test`,
  `--panel-click-proof`, CLI batch runs — stays bound to the registered fixture so an
  evidence run can never move an owner's real windows.
- The single-display work area and the "no window titles are ever read" rule
  ([ADR-0001](./0001-product-boundary.md)) are unchanged.

### 4. Master selection

Extends ADR-0005 §5: the window focused at the time of the explicit command (the
frontmost app's focused window) is the master; the planner never raises or focuses a
window.

## Consequences

### Positive

- The product's actual purpose (tiling real app windows on an explicit command)
  becomes reachable for the first time, with the owner's strict acceptance recorded.
- Preflight read-before-write refuses more failures before touching anything, which is
  the owner's stated preference ("lebih sering menolak penuh").
- Evidence paths stay fixture-bound, so proofs remain safe to run on the owner's host.

### Negative / accepted limitations

- Strict no-partial is a product promise, not a platform guarantee: an unverifiable
  restore after a genuine mid-batch failure surfaces as a loud partial-state status
  the owner must act on (the residual race is proven, Paths F–H).
- More refusals: an app that cannot be enumerated or whose window cannot be read
  blocks the whole reflow (before any write).
- Windows of every regular app move in one batch — mis-chosen state (e.g. a window the
  owner expected to stay) is restored only by an explicit follow-up command, not
  automatically.
- Single display only (existing documented limit).

## Alternatives rejected

- **Scope (b), rules-only apps:** too narrow for a tiling shell; every app would need
  manual registration before any reflow.
- **Scope (c), frontmost app only:** a useful stepping stone but does not deliver the
  tiling-WM behavior the product is for.
- **Best-effort (ADR-0005 semantics) for real windows:** explicitly declined by the
  owner in favor of strict no-partial.
- **Auto-tiling on real windows:** stays off; outside this decision.

## References

- Supersedes-for-productization: [ADR-0005 addendum](./0005-batch-multiwindow-best-effort-rollback.md)
- Superseded: [ADR-0004](./0004-batch-multiwindow-deferred-no-partial.md)
- Evidence: [`docs/ARENA-VERIFICATION.md`](../ARENA-VERIFICATION.md) Paths F–H (mid-batch partial, rollback) and K–M (panel/rows)
- Apple docs: `AXUIElementCopyAttributeValue`, `AXUIElementSetAttributeValue`
