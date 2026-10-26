//! OS-neutral snapshots and pure planning over a single display.
//!
//! These values contain no native handles, window titles, process pointers, or
//! serialized identifiers. Runtime IDs are scoped to one observed session.
//! This module only creates plans; it does not apply geometry or focus actions.

use crate::{
    compute_layout_constrained, ConstrainedWindow, LayoutConfig, LayoutDiagnostic, LayoutError,
    LayoutKind, Placement, Rect, WindowConstraints, WindowId,
};
use std::collections::HashSet;
use std::fmt;

/// Opaque display identity, valid only for the lifetime of an adapter snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DisplayId(pub u64);

/// Classification supplied by a platform adapter before user rules are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    /// Ordinary application window.
    Application,
    /// Dialog owned by an ordinary application; excluded unless explicitly allowed.
    Dialog,
    /// Window belonging to this controller. Always excluded.
    OwnApplication,
    /// OS/system UI. Always excluded.
    System,
    /// Desktop/background surface. Always excluded.
    Desktop,
}

/// Per-operation support reported by the native adapter for this snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowCapabilities {
    pub can_move: bool,
    pub can_resize: bool,
    pub can_focus: bool,
}

/// A title-free, handle-free view of one window at one point in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSnapshot {
    pub id: WindowId,
    pub display: DisplayId,
    pub frame: Rect,
    pub constraints: WindowConstraints,
    pub role: WindowRole,
    pub capabilities: WindowCapabilities,
}

/// Display geometry in logical coordinates. `work_area` excludes OS-reserved
/// regions when the platform can report them through a public API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaySnapshot {
    pub id: DisplayId,
    pub bounds: Rect,
    pub work_area: Rect,
    pub scale_factor: f64,
}

/// A coherent, ephemeral platform observation. `revision` is adapter-local and
/// must not be persisted or treated as a native handle.
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformSnapshot {
    pub revision: u64,
    pub displays: Vec<DisplaySnapshot>,
    /// Stable adapter order, except that `focused_window` is promoted to master
    /// for a plan. The planner never raises or focuses a window.
    pub windows: Vec<WindowSnapshot>,
    pub focused_window: Option<WindowId>,
}

/// Read-only boundary implemented by native adapters and test fakes.
pub trait PlatformSnapshotPort {
    fn current_snapshot(&self) -> Result<PlatformSnapshot, SnapshotReadError>;
}

/// Safe, typed failure to obtain a snapshot. Native error details stay in the
/// adapter and must not contain titles, tokens, or other sensitive data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotReadError {
    Unavailable,
    PermissionDenied,
    StaleObservation,
}

impl fmt::Display for SnapshotReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => write!(f, "platform snapshot unavailable"),
            Self::PermissionDenied => write!(f, "platform snapshot permission denied"),
            Self::StaleObservation => write!(f, "platform snapshot is stale"),
        }
    }
}

impl std::error::Error for SnapshotReadError {}

/// Why an observed window was not included in a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionReason {
    OwnApplication,
    SystemWindow,
    DesktopWindow,
    DialogPolicy,
    /// A matching user rule excluded this window (app protection, ADR-0006).
    /// Applied only after the mandatory gates above — a rule can never
    /// re-include an own/system/desktop window.
    RuleExcluded,
    MoveUnsupported,
    ResizeUnsupported,
    InvalidFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcludedWindow {
    pub window: WindowId,
    pub reason: ExclusionReason,
}

/// Per-window override derived from a user rule (ADR-0006).
///
/// - `exclude` excludes this window from tiling regardless of its role — the
///   app-protection path: a matching `exclude` rule protects an app's normal
///   windows as well as its dialogs.
/// - `include` overrides the dialog include policy for a `Dialog` window;
///   `None` follows the global `include_dialogs` policy. It is ignored for a
///   window with `exclude`.
///
/// Overrides are evaluated **after** the mandatory gates: a rule can never
/// make an `OwnApplication`, `System`, or `Desktop` window tileable, nor an
/// unsupported or invalid window valid. "Rules cannot unlock mandatory" holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRuleOverride {
    pub window: WindowId,
    pub exclude: bool,
    pub include: Option<bool>,
}

/// A pure desired-geometry result. It contains no requests to move, resize,
/// raise, hide, minimize, switch workspaces, or otherwise mutate native state.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPlan {
    pub source_revision: u64,
    pub display: DisplayId,
    pub kind: LayoutKind,
    pub placements: Vec<Placement>,
    pub diagnostics: Vec<LayoutDiagnostic>,
    pub excluded: Vec<ExcludedWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    Snapshot(SnapshotReadError),
    DisplayNotFound(DisplayId),
    InvalidDisplay(DisplayId),
    DuplicateWindowId(WindowId),
    InvalidConstraints(WindowId),
    Layout(LayoutError),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => write!(f, "{error}"),
            Self::DisplayNotFound(id) => write!(f, "display {:?} was not observed", id),
            Self::InvalidDisplay(id) => write!(f, "display {:?} has invalid geometry", id),
            Self::DuplicateWindowId(id) => write!(f, "duplicate runtime window ID {:?}", id),
            Self::InvalidConstraints(id) => {
                write!(f, "window {:?} has non-finite or negative minimum size", id)
            }
            Self::Layout(error) => write!(f, "layout planning failed: {error}"),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<LayoutError> for PlanError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

/// Obtain one snapshot and compute a plan without any platform side effects.
pub fn plan_from_port<P: PlatformSnapshotPort>(
    port: &P,
    display: DisplayId,
    kind: LayoutKind,
    config: LayoutConfig,
    include_dialogs: bool,
) -> Result<LayoutPlan, PlanError> {
    let snapshot = port.current_snapshot().map_err(PlanError::Snapshot)?;
    plan_snapshot(&snapshot, display, kind, config, include_dialogs)
}

/// Compute a deterministic plan from an already captured snapshot.
///
/// The focused eligible window is first (master); all other eligible windows
/// retain the adapter's stable input order. Focus is an ordering input only:
/// the returned plan never requests an implicit raise or focus change.
pub fn plan_snapshot(
    snapshot: &PlatformSnapshot,
    display_id: DisplayId,
    kind: LayoutKind,
    config: LayoutConfig,
    include_dialogs: bool,
) -> Result<LayoutPlan, PlanError> {
    plan_snapshot_inner(snapshot, display_id, kind, config, include_dialogs, None)
}

/// [`plan_snapshot`] with per-window overrides derived from user rules (see
/// [`WindowRuleOverride`]): a matching `exclude` rule removes that window from
/// tiling (app protection), and a dialog-policy override narrows or widens the
/// include decision for a `Dialog` window. Windows without a matching override
/// follow the global `include_dialogs` policy. Mandatory exclusions are
/// evaluated before any override and cannot be widened.
pub fn plan_snapshot_with_rules(
    snapshot: &PlatformSnapshot,
    display_id: DisplayId,
    kind: LayoutKind,
    config: LayoutConfig,
    include_dialogs: bool,
    window_rules: &[WindowRuleOverride],
) -> Result<LayoutPlan, PlanError> {
    plan_snapshot_inner(
        snapshot,
        display_id,
        kind,
        config,
        include_dialogs,
        Some(window_rules),
    )
}

fn plan_snapshot_inner(
    snapshot: &PlatformSnapshot,
    display_id: DisplayId,
    kind: LayoutKind,
    config: LayoutConfig,
    include_dialogs: bool,
    window_rules: Option<&[WindowRuleOverride]>,
) -> Result<LayoutPlan, PlanError> {
    let display = snapshot
        .displays
        .iter()
        .find(|display| display.id == display_id)
        .copied()
        .ok_or(PlanError::DisplayNotFound(display_id))?;
    validate_display(display)?;

    let mut seen = HashSet::with_capacity(snapshot.windows.len());
    if let Some(duplicate) = snapshot
        .windows
        .iter()
        .find(|window| !seen.insert(window.id))
    {
        return Err(PlanError::DuplicateWindowId(duplicate.id));
    }

    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    for window in snapshot.windows.iter().filter(|w| w.display == display_id) {
        let reason = exclusion_reason(window, include_dialogs, window_rules);
        if let Some(reason) = reason {
            excluded.push(ExcludedWindow {
                window: window.id,
                reason,
            });
            continue;
        }
        if !window.frame.is_finite_and_positive() {
            excluded.push(ExcludedWindow {
                window: window.id,
                reason: ExclusionReason::InvalidFrame,
            });
            continue;
        }
        if !valid_constraints(window.constraints) {
            return Err(PlanError::InvalidConstraints(window.id));
        }
        eligible.push(*window);
    }

    if let Some(focused) = snapshot.focused_window {
        if let Some(index) = eligible.iter().position(|window| window.id == focused) {
            let focused_window = eligible.remove(index);
            eligible.insert(0, focused_window);
        }
    }

    let constrained: Vec<ConstrainedWindow> = eligible
        .iter()
        .map(|window| ConstrainedWindow {
            id: window.id,
            constraints: window.constraints,
        })
        .collect();
    let result = compute_layout_constrained(display.work_area, &constrained, kind, config)?;

    Ok(LayoutPlan {
        source_revision: snapshot.revision,
        display: display_id,
        kind,
        placements: result.placements,
        diagnostics: result.diagnostics,
        excluded,
    })
}

fn validate_display(display: DisplaySnapshot) -> Result<(), PlanError> {
    if !display.bounds.is_finite_and_positive()
        || !display.work_area.is_finite_and_positive()
        || !display.bounds.contains(display.work_area, 1e-6)
        || !display.scale_factor.is_finite()
        || display.scale_factor <= 0.0
    {
        return Err(PlanError::InvalidDisplay(display.id));
    }
    Ok(())
}

fn valid_constraints(constraints: WindowConstraints) -> bool {
    constraints.min_width.is_finite()
        && constraints.min_height.is_finite()
        && constraints.min_width >= 0.0
        && constraints.min_height >= 0.0
}

fn exclusion_reason(
    window: &WindowSnapshot,
    include_dialogs: bool,
    window_rules: Option<&[WindowRuleOverride]>,
) -> Option<ExclusionReason> {
    // Mandatory gates first: no rule override is consulted for these, so a
    // rule can never unlock own/system/desktop windows ("rules cannot unlock
    // mandatory" holds by evaluation order).
    match window.role {
        WindowRole::OwnApplication => return Some(ExclusionReason::OwnApplication),
        WindowRole::System => return Some(ExclusionReason::SystemWindow),
        WindowRole::Desktop => return Some(ExclusionReason::DesktopWindow),
        WindowRole::Dialog | WindowRole::Application => {}
    }
    let rule = window_rules
        .unwrap_or(&[])
        .iter()
        .find(|rule| rule.window == window.id);
    // App protection (ADR-0006): a matching `exclude` rule removes this
    // window from tiling whatever its role — normal windows as well as
    // dialogs, after the mandatory gates above.
    if rule.is_some_and(|rule| rule.exclude) {
        return Some(ExclusionReason::RuleExcluded);
    }
    if window.role == WindowRole::Dialog {
        let include = rule
            .and_then(|rule| rule.include)
            .unwrap_or(include_dialogs);
        if !include {
            return Some(ExclusionReason::DialogPolicy);
        }
    }
    if !window.capabilities.can_move {
        return Some(ExclusionReason::MoveUnsupported);
    }
    if !window.capabilities.can_resize {
        return Some(ExclusionReason::ResizeUnsupported);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakePlatformAdapter {
        result: Result<PlatformSnapshot, SnapshotReadError>,
    }

    impl PlatformSnapshotPort for FakePlatformAdapter {
        fn current_snapshot(&self) -> Result<PlatformSnapshot, SnapshotReadError> {
            self.result.clone()
        }
    }

    const DISPLAY: DisplayId = DisplayId(7);

    fn display() -> DisplaySnapshot {
        DisplaySnapshot {
            id: DISPLAY,
            bounds: Rect::new(0.0, 0.0, 1200.0, 800.0),
            work_area: Rect::new(0.0, 24.0, 1200.0, 776.0),
            scale_factor: 2.0,
        }
    }

    fn window(id: u64, role: WindowRole) -> WindowSnapshot {
        WindowSnapshot {
            id: WindowId(id),
            display: DISPLAY,
            frame: Rect::new(30.0, 50.0, 400.0, 300.0),
            constraints: WindowConstraints::UNBOUNDED,
            role,
            capabilities: WindowCapabilities {
                can_move: true,
                can_resize: true,
                can_focus: true,
            },
        }
    }

    fn snapshot(
        windows: Vec<WindowSnapshot>,
        focused_window: Option<WindowId>,
    ) -> PlatformSnapshot {
        PlatformSnapshot {
            revision: 42,
            displays: vec![display()],
            windows,
            focused_window,
        }
    }

    #[test]
    fn focused_window_is_master_and_remaining_order_is_stable() {
        let state = snapshot(
            vec![
                window(1, WindowRole::Application),
                window(2, WindowRole::Application),
                window(3, WindowRole::Application),
            ],
            Some(WindowId(2)),
        );
        let plan = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::MasterStack,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(
            plan.placements.iter().map(|p| p.window).collect::<Vec<_>>(),
            vec![WindowId(2), WindowId(1), WindowId(3)]
        );
        assert_eq!(plan.source_revision, 42);
    }

    #[test]
    fn dialog_rule_include_overrides_global_exclude() {
        let mut window = window(4, WindowRole::Dialog);
        window.frame = Rect::new(0.0, 0.0, 100.0, 100.0);
        let state = snapshot(vec![window], Some(WindowId(4)));
        let plan = plan_snapshot_with_rules(
            &state,
            DISPLAY,
            LayoutKind::MasterStack,
            default_layout(),
            false,
            &[WindowRuleOverride {
                window: WindowId(4),
                exclude: false,
                include: Some(true),
            }],
        )
        .unwrap();
        assert_eq!(plan.placements.len(), 1);
        assert_eq!(plan.placements[0].window, WindowId(4));
    }

    #[test]
    fn dialog_rule_exclude_overrides_global_include() {
        let mut window = window(4, WindowRole::Dialog);
        window.frame = Rect::new(0.0, 0.0, 100.0, 100.0);
        let state = snapshot(vec![window], Some(WindowId(4)));
        let plan = plan_snapshot_with_rules(
            &state,
            DISPLAY,
            LayoutKind::MasterStack,
            default_layout(),
            true,
            &[WindowRuleOverride {
                window: WindowId(4),
                exclude: false,
                include: Some(false),
            }],
        )
        .unwrap();
        assert!(plan.placements.is_empty());
        assert!(
            plan.excluded
                .iter()
                .any(|item| item.window == WindowId(4)
                    && item.reason == ExclusionReason::DialogPolicy)
        );
    }

    #[test]
    fn rules_cannot_unlock_mandatory_exclusions() {
        // A mandatory window (own/system/desktop) stays excluded whatever the
        // override says: both the `include` widening and the `exclude` flag
        // are evaluated after the mandatory gates.
        let roles = [
            (1, WindowRole::OwnApplication),
            (2, WindowRole::System),
            (3, WindowRole::Desktop),
        ];
        let overrides = [
            WindowRuleOverride {
                window: WindowId(0),
                exclude: false,
                include: Some(true),
            },
            WindowRuleOverride {
                window: WindowId(0),
                exclude: true,
                include: None,
            },
        ];
        for (id, role) in roles {
            let mut window = window(id, role);
            window.frame = Rect::new(0.0, 0.0, 100.0, 100.0);
            let state = snapshot(vec![window], None);
            for override_rule in overrides {
                let mut override_rule = override_rule;
                override_rule.window = WindowId(id);
                let plan = plan_snapshot_with_rules(
                    &state,
                    DISPLAY,
                    LayoutKind::MasterStack,
                    default_layout(),
                    false,
                    &[override_rule],
                )
                .unwrap();
                assert!(plan.placements.is_empty());
                assert!(plan.excluded.iter().any(|item| item.window == WindowId(id)));
            }
        }
    }

    #[test]
    fn exclude_rule_excludes_a_normal_app_window() {
        // App protection (ADR-0006): `exclude` removes a normal window from
        // tiling, not only dialogs.
        let mut app = window(6, WindowRole::Application);
        app.frame = Rect::new(10.0, 10.0, 100.0, 100.0);
        let mut other = window(7, WindowRole::Application);
        other.frame = Rect::new(120.0, 10.0, 100.0, 100.0);
        let state = snapshot(vec![app, other], Some(WindowId(6)));
        let plan = plan_snapshot_with_rules(
            &state,
            DISPLAY,
            LayoutKind::MasterStack,
            default_layout(),
            false,
            &[WindowRuleOverride {
                window: WindowId(7),
                exclude: true,
                include: None,
            }],
        )
        .unwrap();
        assert_eq!(plan.placements.len(), 1);
        assert_eq!(plan.placements[0].window, WindowId(6));
        assert!(
            plan.excluded
                .iter()
                .any(|item| item.window == WindowId(7)
                    && item.reason == ExclusionReason::RuleExcluded)
        );
    }

    #[test]
    fn exclude_rule_wins_over_dialog_include_for_a_dialog() {
        let mut dialog = window(4, WindowRole::Dialog);
        dialog.frame = Rect::new(0.0, 0.0, 100.0, 100.0);
        let state = snapshot(vec![dialog], Some(WindowId(4)));
        let plan = plan_snapshot_with_rules(
            &state,
            DISPLAY,
            LayoutKind::MasterStack,
            default_layout(),
            true,
            &[WindowRuleOverride {
                window: WindowId(4),
                exclude: true,
                include: Some(true),
            }],
        )
        .unwrap();
        assert!(plan.placements.is_empty());
        assert!(
            plan.excluded
                .iter()
                .any(|item| item.window == WindowId(4)
                    && item.reason == ExclusionReason::RuleExcluded)
        );
    }

    #[test]
    fn plan_snapshot_with_empty_rules_matches_plan_snapshot() {
        let mut dialog = window(4, WindowRole::Dialog);
        dialog.frame = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut app = window(6, WindowRole::Application);
        app.frame = Rect::new(10.0, 10.0, 100.0, 100.0);
        let state = snapshot(vec![dialog, app], Some(WindowId(6)));
        let without_rules = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Columns,
            default_layout(),
            false,
        )
        .unwrap();
        let with_empty = plan_snapshot_with_rules(
            &state,
            DISPLAY,
            LayoutKind::Columns,
            default_layout(),
            false,
            &[],
        )
        .unwrap();
        assert_eq!(without_rules, with_empty);
    }

    fn default_layout() -> LayoutConfig {
        crate::LayoutConfig::default()
    }

    #[test]
    fn mandatory_exclusions_and_unsupported_operations_are_reported() {
        let mut cannot_move = window(5, WindowRole::Application);
        cannot_move.capabilities.can_move = false;
        let state = snapshot(
            vec![
                window(1, WindowRole::OwnApplication),
                window(2, WindowRole::System),
                window(3, WindowRole::Desktop),
                window(4, WindowRole::Dialog),
                cannot_move,
                window(6, WindowRole::Application),
            ],
            None,
        );
        let plan = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Columns,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(
            plan.placements.iter().map(|p| p.window).collect::<Vec<_>>(),
            vec![WindowId(6)]
        );
        assert_eq!(
            plan.excluded,
            vec![
                ExcludedWindow {
                    window: WindowId(1),
                    reason: ExclusionReason::OwnApplication
                },
                ExcludedWindow {
                    window: WindowId(2),
                    reason: ExclusionReason::SystemWindow
                },
                ExcludedWindow {
                    window: WindowId(3),
                    reason: ExclusionReason::DesktopWindow
                },
                ExcludedWindow {
                    window: WindowId(4),
                    reason: ExclusionReason::DialogPolicy
                },
                ExcludedWindow {
                    window: WindowId(5),
                    reason: ExclusionReason::MoveUnsupported
                },
            ]
        );
    }

    #[test]
    fn dialog_can_be_included_only_by_explicit_policy() {
        let state = snapshot(vec![window(1, WindowRole::Dialog)], None);
        let excluded = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Monocle,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        let included = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Monocle,
            LayoutConfig::default(),
            true,
        )
        .unwrap();
        assert!(excluded.placements.is_empty());
        assert_eq!(included.placements.len(), 1);
    }

    #[test]
    fn invalid_constraints_fail_before_layout_geometry() {
        let mut invalid = window(9, WindowRole::Application);
        invalid.constraints.min_width = f64::NAN;
        let state = snapshot(vec![invalid], None);
        assert_eq!(
            plan_snapshot(
                &state,
                DISPLAY,
                LayoutKind::Bsp,
                LayoutConfig::default(),
                false,
            ),
            Err(PlanError::InvalidConstraints(WindowId(9)))
        );
    }

    #[test]
    fn invalid_display_geometry_is_rejected() {
        let mut state = snapshot(vec![], None);
        state.displays[0].scale_factor = f64::INFINITY;
        assert_eq!(
            plan_snapshot(
                &state,
                DISPLAY,
                LayoutKind::Bsp,
                LayoutConfig::default(),
                false,
            ),
            Err(PlanError::InvalidDisplay(DISPLAY))
        );
    }

    #[test]
    fn plan_is_deterministic_and_contains_no_focus_action() {
        let state = snapshot(
            vec![
                window(1, WindowRole::Application),
                window(2, WindowRole::Application),
            ],
            Some(WindowId(1)),
        );
        let first = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Bsp,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        let second = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Bsp,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn fake_snapshot_port_drives_the_same_pure_planner() {
        let port = FakePlatformAdapter {
            result: Ok(snapshot(vec![window(1, WindowRole::Application)], None)),
        };
        let plan = plan_from_port(
            &port,
            DISPLAY,
            LayoutKind::Monocle,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(plan.placements.len(), 1);
        assert_eq!(plan.source_revision, 42);
    }

    #[test]
    fn snapshot_port_failure_is_preserved() {
        let port = FakePlatformAdapter {
            result: Err(SnapshotReadError::PermissionDenied),
        };
        assert_eq!(
            plan_from_port(
                &port,
                DISPLAY,
                LayoutKind::Monocle,
                LayoutConfig::default(),
                false,
            ),
            Err(PlanError::Snapshot(SnapshotReadError::PermissionDenied))
        );
    }

    #[test]
    fn other_display_windows_do_not_enter_this_plan() {
        let mut other = window(2, WindowRole::Application);
        other.display = DisplayId(8);
        let state = snapshot(vec![window(1, WindowRole::Application), other], None);
        let plan = plan_snapshot(
            &state,
            DISPLAY,
            LayoutKind::Columns,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(plan.placements.len(), 1);
        assert!(plan.excluded.is_empty());
    }
}
