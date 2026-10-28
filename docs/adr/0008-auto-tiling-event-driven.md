# ADR-0008: Event-Driven Auto-Tiling (macOS) via AXObserver — Opt-In, One Action

- **Status:** Accepted (owner decisions 2026-10-28, recorded Q&A — see "Owner decisions" below)
- **Date:** 2026-10-28
- **Decision owners:** Product + owner
- **Relates to:** [ADR-0006](./0006-real-window-reflow-strict-no-partial.md) (the strict reflow command auto-tiling triggers — this ADR adds no second tiling code path), [ADR-0007](./0007-global-hotkey-reflow.md) (the other non-fixture trigger), [ADR-0001](./0001-product-boundary.md), docs/PRD.md ("when I enable auto-tiling, eligible windows should be arranged by my selected layout"), docs/M2-KEYBOARD-PLAN.md (whose "no auto-tiling" bound was **scope-bound to M2** and is superseded here for the shell surface, exactly as the hotkey rule was in ADR-0007)

## Context

The config already carries `auto_tiling` (default `false`, parsed, unit-tested), the
settings panel already carries an "Auto-tiling" checkbox, and the `settings:` log line
already reports `auto-tiling=on/off` — but **nothing consumes the switch**; every reflow
so far (host-validated, Path O) was started by a hand: a menu click or the ⌥⌘R hotkey.
The delivery roadmap lists event-driven auto-tiling as the next outstanding Phase 2
requirement, and the PRD binds it opt-in: off until the user explicitly enables it.

The owner's standing constraints apply unchanged: strict no-partial (ADR-0006) is the
product promise, rules protect apps, public APIs only, no new TCC permission, no new
dependency without an ADR, and automated evidence stays fixture-bound.

**Owner decisions (2026-10-28, recorded Q&A — all four were owner picks):**

1. **Trigger:** window open + window close only — user moves/resizes are honored, never
   snapped back.
2. **Refusal in automatic mode:** loud status + full rollback, then wait for the next
   event — no auto-retry loop, no auto-disable.
3. **Enable/disable path:** the existing panel checkbox + Save, applied live — no
   relaunch.
4. **Apps with an `exclude` rule trigger nothing** — they behave like floating windows.

## Decision

### 1. Opt-in only; the existing checkbox is the on-switch, applied live

- `auto_tiling` stays default `false`. Enabled = `true` in the config, set through the
  panel checkbox + Save (the existing row-based save path) or by editing the file.
- Enabling applies **live**: observers attach immediately and one re-tile runs at once
  (the same strict command) so the owner sees the effect without hunting for a window
  event. Disabling detaches every observer immediately; quit detaches everything and
  leaves windows at their last position (product rule unchanged).
- No new menu item, no new UI surface: the checkbox and the `auto-tiling=on/off` status
  line are the whole onboarding.

### 2. Trigger = window open + close, observed through public AX APIs

- Per-app **`AXObserver`** (public Accessibility API — the same TCC trust the shell
  already holds; **no new permission**): `kAXWindowCreatedNotification` on the app
  element, `kAXUIElementDestroyedNotification` on each observed window element.
- **`NSWorkspace` app notifications** (`didLaunch` / `didTerminateApplication`) attach
  the per-app observer to every regular app that launches and drop it when the app dies;
  on enable, observers attach to all regular apps currently running.
- **`kAXMovedNotification` / `kAXResizedNotification` are deliberately not subscribed**
  — owner drags are honored (decision 1), and this is also the invariant that makes a
  feedback loop impossible: **our own AX writes emit moved/resized notifications on
  observed windows, but geometry writes never emit created/destroyed**, so the automatic
  path cannot be re-triggered by its own writes.
- **Excluded apps are never observed** (decision 4): an app with an `exclude` rule gets
  no observer, so its window churn neither moves other windows nor produces events —
  app protection now also means *no influence*, not just *no writes*.
- The automatic path targets exactly the eligible set the manual command computes
  (mandatory gates, rules, dialogs policy — ADR-0006 unchanged).

### 3. One event burst → one action: the same strict command, coalesced

- Notifications arrive in bursts (a launch sequence can emit several created/destroyed
  events). A **coalescer** collapses any burst into **one scheduled re-tile** with a
  short spacing window (~500 ms, single pending timer on the main run loop, latest
  state wins). No queue, no per-event action.
- The scheduled action is **`perform_reflow_real` itself** — the identical function the
  menu click and the ⌥⌘R hotkey call: same preflight, same strict no-partial contract,
  same rules, same three-way status, same log line. There is no second tiling code path
  and no separate semantics; an automatic run is indistinguishable in the log from a
  manual one.
- **Paused gate:** while paused, events are dropped silently (no re-tile, no status
  churn) — the chip already tells the owner the shell is paused.
- **Refusal (decision 2):** the automatic run reports the same loud
  `refused (…) — all windows restored` status a manual run produces and simply waits;
  nothing retries and nothing disables itself. The remedy loop stays owner-driven
  (exclude the clamping app, as done for Nicegram and System Settings).

### 4. Lifecycle and failure behavior

- All observer work happens on the main thread/run loop — the same thread as the Carbon
  hotkey handler and menu actions. No new threads, no new run loops.
- Observer creation or notification-attach failure for one app is logged **once**, that
  app is skipped as a *trigger source* (manual reflow still targets it per rules); the
  automatic path degrades to fewer triggers, never to wrong writes.
- A hung app simply stops delivering events (best-effort observation); the next event
  from any app re-syncs the layout. No polling fallback — polling would be its own
  decision.
- Teardown is explicit on disable and quit (remove notifications, release observer
  refs); the OS would release them at exit anyway, but explicit teardown keeps the
  lifecycle honest (same stance as ADR-0007 unregister).

### 5. Evidence expectations and scope

- **Pure parts unit-tested** (no Mac needed): the coalescer (burst → one tick), the
  trigger filter (excluded app → no event), the enable/disable state machine
  (attach/detach decisions, quit teardown), and the invariant "automatic action calls
  `perform_reflow_real`, never the fixture path".
- **Host evidence = a new ARENA path (Q), owner-driven:** enable via the panel, open a
  window, observe the automatic `tiled N window(s)` in the log; close it, observe the
  second automatic run; a clamping app produces the loud refusal with rollback; disable
  → no further automatic lines. Automated paths (`--self-test`, `--panel-click-proof`)
  **never attach observers**, keeping automated evidence exactly as it is.
- **macOS only.** Windows auto-tiling is out of scope here (the adapter interface stays
  platform-neutral, as always).
- Implementation lands as **separate PRs after this ADR merges** (one change per
  session rule): FFI declarations (`AXObserver*` + notification constants, every
  `unsafe` justified), coalescer + observer manager (pure + wired), shell wiring
  (enable path, NSWorkspace notifications), then the host proof.

## Consequences

- Reflow gains a third trigger whose initiator is the *system* — accepted because the
  trigger is standing owner consent (the toggle), each individual action still runs the
  owner-approved strict command, and the PRD explicitly wants this behavior when
  enabled.
- The status line and log get busier while enabled: one line per coalesced automatic
  action (same format as manual) plus one attach-failure line per problem app —
  accepted as the honest price of observability.
- An excluded app may visually sit on top of a tiled layout without ever triggering a
  re-tile and without being moved — accepted (floating-like), consistent with decision 4.
- If a specific app's window never reports destroy (observer quirk), the layout stays
  stale until the next event from any observed app — accepted; a fix would be its own
  decision (e.g. app-launch re-sync only).
- `auto_tiling: true` in a hand-edited config now has real effect on the next launch —
  accepted; the file was always owner-controlled (0600) and the panel remains the
  recommended path.
