//! Pure decision logic of event-driven auto-tiling —
//! [ADR-0008](../../docs/adr/0008-auto-tiling-event-driven.md).
//!
//! This module holds the unit-testable parts only: the **coalescer** (a
//! burst of window open/close notifications collapses into at most one
//! re-tile per tick) and the **trigger filter** (an app with an `exclude`
//! rule triggers nothing — ADR-0008 decision 4). The AXObserver wiring
//! that feeds these — attach/detach on the main run loop, `NSWorkspace`
//! app notifications, the timer, the paused gate — lives in the shell and
//! lands in a later PR; until then nothing in the crate calls this module,
//! which is exactly that PR's "no behavior change" contract. The automatic
//! action is and stays `perform_reflow_real` itself (ADR-0008 §3).

use crate::config::{DialogPolicy, WindowRule};

/// Collapses a burst of window open/close events into one scheduled action.
///
/// ADR-0008 §3: notifications arrive in bursts (launch sequences, dialogs
/// spawning windows). Any number of [`on_event`](Self::on_event) calls
/// before the coalescing timer fires yield **at most one** `true` from
/// [`take_tick`](Self::take_tick) — one re-tile per burst, latest state
/// wins, no queue. The timer itself (a single pending ~500 ms main-run-loop
/// timer) is wiring, not logic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coalescer {
    pending: bool,
}

impl Coalescer {
    /// A quiet coalescer: no event since the last tick.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a window open/close event. Pure state — starting or extending
    /// the coalescing timer is the wiring's job.
    pub fn on_event(&mut self) {
        self.pending = true;
    }

    /// The coalescing timer fired. Returns `true` when at least one event
    /// arrived since the previous fire — the caller then runs exactly one
    /// re-tile. A second fire without a new event returns `false`.
    pub fn take_tick(&mut self) -> bool {
        std::mem::replace(&mut self.pending, false)
    }
}

/// ADR-0008 decision 4: whether the app identified by `pid`/`bundle` may
/// trigger an automatic re-tile. An app matched by an `exclude` rule (the
/// ADR-0006 app-protection policy, `dialogs: Exclude`) triggers nothing —
/// it behaves like a floating window. Rules with `Include`/`Default`
/// policy never block triggering, and a rule that does not match the
/// source (e.g. a bundle rule against an app without a bundle id) leaves
/// triggering allowed.
pub fn app_may_trigger(rules: &[WindowRule], pid: u32, bundle: Option<&str>) -> bool {
    !rules
        .iter()
        .any(|rule| rule.dialogs == DialogPolicy::Exclude && rule.matches(pid, bundle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exclude(bundle: &str) -> WindowRule {
        WindowRule {
            bundle: Some(bundle.to_string()),
            pid: None,
            dialogs: DialogPolicy::Exclude,
        }
    }

    #[test]
    fn coalescer_fires_once_per_burst_and_stays_quiet_until_the_next_event() {
        let mut coalescer = Coalescer::new();
        // Nothing happened: the first tick is quiet.
        assert!(!coalescer.take_tick());
        // A burst of three events is one action, and only one.
        coalescer.on_event();
        coalescer.on_event();
        coalescer.on_event();
        assert!(coalescer.take_tick());
        assert!(!coalescer.take_tick());
        // A later, separate event starts a new burst.
        coalescer.on_event();
        assert!(coalescer.take_tick());
        assert!(!coalescer.take_tick());
    }

    #[test]
    fn coalescer_defaults_to_quiet() {
        assert!(!Coalescer::default().take_tick());
    }

    #[test]
    fn excluded_app_triggers_nothing_and_ordinary_apps_trigger() {
        let rules = vec![exclude("com.mobyrix.Nicegram")];
        assert!(!app_may_trigger(&rules, 477, Some("com.mobyrix.Nicegram")));
        assert!(app_may_trigger(&rules, 42, Some("com.apple.finder")));
        // No rules at all: every regular app may trigger.
        assert!(app_may_trigger(&[], 42, Some("com.apple.finder")));
    }

    #[test]
    fn include_and_default_rules_never_block_triggering() {
        let rules = vec![
            WindowRule {
                bundle: Some("com.a".to_string()),
                pid: None,
                dialogs: DialogPolicy::Include,
            },
            WindowRule {
                bundle: Some("com.b".to_string()),
                pid: None,
                dialogs: DialogPolicy::Default,
            },
        ];
        assert!(app_may_trigger(&rules, 1, Some("com.a")));
        assert!(app_may_trigger(&rules, 1, Some("com.b")));
    }

    #[test]
    fn pid_exclude_rule_matches_by_pid_only() {
        let rules = vec![WindowRule {
            bundle: None,
            pid: Some(900),
            dialogs: DialogPolicy::Exclude,
        }];
        assert!(!app_may_trigger(&rules, 900, None));
        assert!(!app_may_trigger(&rules, 900, Some("com.anything")));
        assert!(app_may_trigger(&rules, 901, None));
    }

    #[test]
    fn bundle_rule_never_matches_an_app_without_a_bundle_id() {
        let rules = vec![exclude("com.a")];
        // Unsigned CLI apps legitimately report no bundle id; the rule
        // matcher stays fail-closed on the bundle side (config.rs).
        assert!(app_may_trigger(&rules, 7, None));
    }
}
