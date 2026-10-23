//! Multi-window batch application with best-effort rollback.
//!
//! Semantics (see [ADR-0005](../adr/0005-batch-multiwindow-best-effort-rollback.md)):
//! a batch applies a verified frame to each target in order. On the first
//! failure the batch stops and every window already changed is restored to its
//! pre-batch frame and verified. Rollback is **best-effort**: if a restore
//! fails, the actual resulting state is reported honestly and the batch never
//! claims atomicity. A failed batch can therefore leave a partial state; the
//! report says so via `rollback_all_restored == false`.
//!
//! The module is pure with respect to the injected [`BatchOps`] seam: the real
//! adapter implements it with AX, and unit tests implement it with scripted
//! outcomes so the rollback contract is verified without a Mac.

use std::collections::HashSet;
use std::path::Path;

use layout_core::{LayoutPlan, Rect, WindowId};

use crate::adapter::{AxError, WindowIdMap};
use crate::ffi;

pub use crate::adapter::WindowKey;

/// One desired frame for one target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchTarget {
    pub window: WindowKey,
    pub rect: Rect,
}

/// Injectable per-window seam. The real adapter implements it with AX
/// ([`crate::adapter::MacWindowSet`]); unit tests implement it with scripted
/// outcomes (no Apple frameworks).
pub trait BatchOps {
    /// Read the current frame of `window`.
    fn read_frame(&self, window: WindowKey) -> Result<Rect, AxError>;
    /// Apply a verified frame to `window` (single-window no-partial contract).
    fn set_frame(&self, window: WindowKey, rect: Rect) -> Result<Rect, AxError>;
    /// Restore `before` after a later failure and report whether the restore
    /// was verified, plus the observed frame (None if it could not be read).
    fn restore(&self, window: WindowKey, before: Rect) -> (bool, Option<Rect>);
}

/// Map a pure layout plan to native batch targets through one observation's
/// ID map.
///
/// Fails closed: a placement whose domain window ID has no native key in the
/// map produces no target list, so nothing is written.
pub fn targets_for_plan(
    plan: &LayoutPlan,
    map: &WindowIdMap,
) -> Result<Vec<BatchTarget>, PlanError> {
    let mut targets = Vec::with_capacity(plan.placements.len());
    for placement in &plan.placements {
        let window = map
            .get(placement.window)
            .ok_or(PlanError::UnmappedWindow(placement.window))?;
        targets.push(BatchTarget {
            window,
            rect: placement.rect,
        });
    }
    Ok(targets)
}

/// Outcome for one window.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowOutcome {
    /// Applied and verified.
    Applied { requested: Rect, actual: Rect },
    /// Restored after a later failure; `actual` is the observed frame after
    /// the restore (None if it could not be read).
    RolledBack {
        restored: Rect,
        actual: Option<Rect>,
    },
}

/// Batch result. `rollback_all_restored` is true only when every restore was
/// verified; when false the batch left a partial state and says so.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchResult {
    pub outcomes: Vec<(WindowKey, WindowOutcome)>,
    /// The target whose failure stopped the batch, if any.
    pub failed_at: Option<WindowKey>,
    /// The failure that stopped the batch, if any.
    pub error: Option<AxError>,
    pub rollback_all_restored: bool,
}

/// Record the final outcome for `window`, replacing any earlier entry (a
/// window that was applied and later rolled back reports only the rollback).
fn record_outcome(
    outcomes: &mut Vec<(WindowKey, WindowOutcome)>,
    window: WindowKey,
    outcome: WindowOutcome,
) {
    if let Some(slot) = outcomes.iter_mut().find(|(key, _)| *key == window) {
        slot.1 = outcome;
    } else {
        outcomes.push((window, outcome));
    }
}

/// Apply `targets` in order with best-effort rollback.
///
/// Each target is read before it is written so a failure always has a known
/// rollback frame. The first failure stops the batch and triggers a restore of
/// every window already changed (including the failed one, whose single-window
/// transaction may have left a partial write).
pub fn apply_batch<O: BatchOps>(ops: &O, targets: &[BatchTarget]) -> BatchResult {
    let mut saved: Vec<(WindowKey, Rect)> = Vec::with_capacity(targets.len());
    let mut outcomes: Vec<(WindowKey, WindowOutcome)> = Vec::new();

    for target in targets {
        match ops.read_frame(target.window) {
            Ok(before) => saved.push((target.window, before)),
            Err(error) => return finish(ops, saved, outcomes, target.window, error),
        }
        match ops.set_frame(target.window, target.rect) {
            Ok(actual) => record_outcome(
                &mut outcomes,
                target.window,
                WindowOutcome::Applied {
                    requested: target.rect,
                    actual,
                },
            ),
            Err(error) => return finish(ops, saved, outcomes, target.window, error),
        }
    }

    BatchResult {
        outcomes,
        failed_at: None,
        error: None,
        rollback_all_restored: true,
    }
}

/// Stop the batch and restore every already-changed window (best effort).
fn finish<O: BatchOps>(
    ops: &O,
    saved: Vec<(WindowKey, Rect)>,
    mut outcomes: Vec<(WindowKey, WindowOutcome)>,
    failed_at: WindowKey,
    error: AxError,
) -> BatchResult {
    let mut all_restored = true;
    for (window, before) in saved {
        let (verified, actual) = ops.restore(window, before);
        if !verified {
            all_restored = false;
        }
        record_outcome(
            &mut outcomes,
            window,
            WindowOutcome::RolledBack {
                restored: before,
                actual,
            },
        );
    }
    BatchResult {
        outcomes,
        failed_at: Some(failed_at),
        error: Some(error),
        rollback_all_restored: all_restored,
    }
}

/// Why a batch plan was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    /// The plan file has no targets.
    Empty,
    /// The same window appears more than once.
    Duplicate(WindowKey),
    /// A frame is not finite with a positive size.
    InvalidGeometry,
    /// A line could not be parsed.
    Parse,
    /// A target key was not observed in the process the batch is authorized
    /// for, so it cannot be resolved to a native window.
    UnknownTarget(WindowKey),
    /// A plan placement has no native key in the observation's ID map.
    UnmappedWindow(WindowId),
    /// The plan file is not owner-only (regular, non-symlink, perm 0600, owned
    /// by the current user).
    NotOwnerOnly,
}

/// Parse a plan: one target per line as `key x y width height`, where `key` is
/// a runtime window key from `ax-probe windows <pid>`. Blank lines and lines
/// starting with `#` are ignored.
pub fn parse_plan(text: &str) -> Result<Vec<BatchTarget>, PlanError> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(PlanError::Parse);
        }
        let key = parts[0].parse::<u64>().map_err(|_| PlanError::Parse)?;
        let mut nums = [0.0_f64; 4];
        for (slot, part) in nums.iter_mut().zip(parts[1..].iter()) {
            *slot = part.parse::<f64>().map_err(|_| PlanError::Parse)?;
        }
        let rect = Rect::new(nums[0], nums[1], nums[2], nums[3]);
        if !rect.is_finite_and_positive() {
            return Err(PlanError::InvalidGeometry);
        }
        let window = WindowKey(key);
        if !seen.insert(window) {
            return Err(PlanError::Duplicate(window));
        }
        targets.push(BatchTarget { window, rect });
    }
    if targets.is_empty() {
        return Err(PlanError::Empty);
    }
    Ok(targets)
}

/// Verify the plan file is owner-only: a regular non-symlink file owned by the
/// current user with perm 0600. This mirrors the fixture-registration guard so
/// a batch cannot be driven by a tampered or world-writable plan.
pub fn verify_plan_file(path: &Path) -> Result<(), PlanError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(path).map_err(|_| PlanError::NotOwnerOnly)?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(PlanError::NotOwnerOnly);
    }
    if meta.uid() != unsafe { ffi::getuid() } {
        return Err(PlanError::NotOwnerOnly);
    }
    if meta.permissions().mode() & 0o777 != 0o600 {
        return Err(PlanError::NotOwnerOnly);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Scripted [`BatchOps`] mock: no Apple frameworks. Keys are runtime
    /// window keys, so several keys can describe windows of one process.
    #[derive(Default)]
    struct MockBatch {
        frames: RefCell<HashMap<u64, Rect>>,
        fail_set: HashSet<u64>,
        fail_read: HashSet<u64>,
        fail_restore: HashSet<u64>,
        writes: RefCell<Vec<(u64, &'static str)>>,
    }

    impl MockBatch {
        fn with(windows: &[(u64, Rect)]) -> Self {
            let mut frames = HashMap::new();
            for (key, rect) in windows {
                frames.insert(*key, *rect);
            }
            Self {
                frames: RefCell::new(frames),
                ..Self::default()
            }
        }
    }

    impl BatchOps for MockBatch {
        fn read_frame(&self, window: WindowKey) -> Result<Rect, AxError> {
            if self.fail_read.contains(&window.0) {
                return Err(AxError::Failure(-25204));
            }
            self.frames
                .borrow()
                .get(&window.0)
                .copied()
                .ok_or(AxError::StaleTarget)
        }

        fn set_frame(&self, window: WindowKey, rect: Rect) -> Result<Rect, AxError> {
            if self.fail_set.contains(&window.0) {
                return Err(AxError::Failure(-25200));
            }
            self.frames.borrow_mut().insert(window.0, rect);
            self.writes.borrow_mut().push((window.0, "set"));
            Ok(rect)
        }

        fn restore(&self, window: WindowKey, before: Rect) -> (bool, Option<Rect>) {
            if self.fail_restore.contains(&window.0) {
                return (false, None);
            }
            self.frames.borrow_mut().insert(window.0, before);
            self.writes.borrow_mut().push((window.0, "restore"));
            (true, Some(before))
        }
    }

    fn target(key: u64, x: f64, y: f64, w: f64, h: f64) -> BatchTarget {
        BatchTarget {
            window: WindowKey(key),
            rect: Rect::new(x, y, w, h),
        }
    }

    #[test]
    fn all_targets_are_applied_in_order() {
        let ops = MockBatch::with(&[
            (1, Rect::new(0.0, 0.0, 100.0, 50.0)),
            (2, Rect::new(0.0, 0.0, 100.0, 50.0)),
        ]);
        let result = apply_batch(
            &ops,
            &[
                target(1, 10.0, 10.0, 200.0, 100.0),
                target(2, 220.0, 10.0, 200.0, 100.0),
            ],
        );
        assert!(result.error.is_none());
        assert!(result.failed_at.is_none());
        assert!(result.rollback_all_restored);
        assert_eq!(
            result.outcomes,
            vec![
                (
                    WindowKey(1),
                    WindowOutcome::Applied {
                        requested: Rect::new(10.0, 10.0, 200.0, 100.0),
                        actual: Rect::new(10.0, 10.0, 200.0, 100.0),
                    }
                ),
                (
                    WindowKey(2),
                    WindowOutcome::Applied {
                        requested: Rect::new(220.0, 10.0, 200.0, 100.0),
                        actual: Rect::new(220.0, 10.0, 200.0, 100.0),
                    }
                ),
            ]
        );
    }

    #[test]
    fn failure_stops_the_batch_and_rolls_back_changed_windows() {
        let mut ops = MockBatch::with(&[
            (1, Rect::new(0.0, 0.0, 100.0, 50.0)),
            (2, Rect::new(0.0, 0.0, 100.0, 50.0)),
        ]);
        ops.fail_set.insert(2);
        let result = apply_batch(
            &ops,
            &[
                target(1, 10.0, 10.0, 200.0, 100.0),
                target(2, 220.0, 10.0, 200.0, 100.0),
            ],
        );
        assert_eq!(result.failed_at, Some(WindowKey(2)));
        assert!(result.error.is_some());
        assert!(result.rollback_all_restored);
        assert_eq!(
            result.outcomes,
            vec![
                (
                    WindowKey(1),
                    WindowOutcome::RolledBack {
                        restored: Rect::new(0.0, 0.0, 100.0, 50.0),
                        actual: Some(Rect::new(0.0, 0.0, 100.0, 50.0)),
                    }
                ),
                (
                    WindowKey(2),
                    WindowOutcome::RolledBack {
                        restored: Rect::new(0.0, 0.0, 100.0, 50.0),
                        actual: Some(Rect::new(0.0, 0.0, 100.0, 50.0)),
                    }
                ),
            ]
        );
        // 1 set + 2 restore writes (window 2's failed set wrote nothing).
        assert_eq!(ops.writes.borrow().len(), 3);
    }

    #[test]
    fn rollback_failure_is_reported_not_hidden() {
        let mut ops = MockBatch::with(&[
            (1, Rect::new(0.0, 0.0, 100.0, 50.0)),
            (2, Rect::new(0.0, 0.0, 100.0, 50.0)),
        ]);
        ops.fail_set.insert(2);
        ops.fail_restore.insert(1);
        let result = apply_batch(
            &ops,
            &[
                target(1, 10.0, 10.0, 200.0, 100.0),
                target(2, 220.0, 10.0, 200.0, 100.0),
            ],
        );
        assert!(!result.rollback_all_restored);
        assert!(matches!(
            result.outcomes[0].1,
            WindowOutcome::RolledBack { actual: None, .. }
        ));
    }

    #[test]
    fn read_failure_before_any_change_needs_no_rollback() {
        let mut ops = MockBatch::with(&[(1, Rect::new(0.0, 0.0, 100.0, 50.0))]);
        ops.fail_read.insert(1);
        let result = apply_batch(&ops, &[target(1, 10.0, 10.0, 200.0, 100.0)]);
        assert_eq!(result.failed_at, Some(WindowKey(1)));
        assert!(result.outcomes.is_empty());
        assert!(result.rollback_all_restored);
        assert!(ops.writes.borrow().is_empty());
    }

    #[test]
    fn empty_batch_is_a_noop_success() {
        let ops = MockBatch::default();
        let result = apply_batch(&ops, &[]);
        assert!(result.error.is_none());
        assert!(result.outcomes.is_empty());
        assert!(result.rollback_all_restored);
    }

    #[test]
    fn parse_plan_accepts_targets_comments_and_blank_lines() {
        let text = "# layout\n1 10 20 400 300\n\n2 420 20 400 300\n";
        let targets = parse_plan(text).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].window, WindowKey(1));
        assert_eq!(targets[0].rect, Rect::new(10.0, 20.0, 400.0, 300.0));
    }

    #[test]
    fn parse_plan_rejects_bad_input() {
        assert_eq!(parse_plan(""), Err(PlanError::Empty));
        assert_eq!(parse_plan("# only a comment\n"), Err(PlanError::Empty));
        assert_eq!(parse_plan("1 10 20 400"), Err(PlanError::Parse));
        assert_eq!(
            parse_plan("1 10 20 400 300\n1 1 1 1 1"),
            Err(PlanError::Duplicate(WindowKey(1)))
        );
        assert_eq!(parse_plan("1 10 20 0 300"), Err(PlanError::InvalidGeometry));
        assert_eq!(
            parse_plan("1 nan 20 400 300"),
            Err(PlanError::InvalidGeometry)
        );
    }

    // MARK: - Plan -> native target mapping (pure seam)

    #[test]
    fn targets_for_plan_maps_two_windows_of_one_process() {
        use crate::adapter::observation_from_windows;
        use layout_core::{DisplayId, LayoutConfig, LayoutKind};

        let display = Rect::new(0.0, 0.0, 1200.0, 800.0);
        // Keys 0 and 1 are two windows of the SAME process.
        let observation = observation_from_windows(
            display,
            &[
                (WindowKey(0), Rect::new(10.0, 10.0, 200.0, 200.0)),
                (WindowKey(1), Rect::new(220.0, 10.0, 200.0, 200.0)),
            ],
            Some(WindowKey(1)),
        );
        let plan = layout_core::plan_snapshot(
            &observation.snapshot,
            DisplayId(0),
            LayoutKind::MasterStack,
            LayoutConfig::default(),
            false,
        )
        .unwrap();
        let targets = targets_for_plan(&plan, &observation.map).unwrap();
        assert_eq!(targets.len(), 2);
        // Focused window is the master (first), the other keeps its order.
        assert_eq!(targets[0].window, WindowKey(1));
        assert_eq!(targets[1].window, WindowKey(0));
    }

    #[test]
    fn targets_for_plan_fails_closed_on_unmapped_id() {
        use crate::adapter::observation_from_windows;
        use layout_core::{DisplayId, LayoutKind, LayoutPlan, Placement, WindowId};

        let observation = observation_from_windows(
            Rect::new(0.0, 0.0, 1200.0, 800.0),
            &[(WindowKey(0), Rect::new(0.0, 0.0, 100.0, 100.0))],
            None,
        );
        let plan = LayoutPlan {
            source_revision: observation.snapshot.revision,
            display: DisplayId(0),
            kind: LayoutKind::Monocle,
            placements: vec![Placement {
                window: WindowId(9),
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            }],
            diagnostics: Vec::new(),
            excluded: Vec::new(),
        };
        assert_eq!(
            targets_for_plan(&plan, &observation.map),
            Err(PlanError::UnmappedWindow(WindowId(9)))
        );
    }

    #[test]
    fn plan_file_must_be_owner_only() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("hypr-batch-plan-{}.txt", std::process::id()));
        {
            let mut file = std::fs::File::create(&path).unwrap();
            writeln!(file, "1 10 20 400 300").unwrap();
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        assert_eq!(verify_plan_file(&path), Ok(()));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(verify_plan_file(&path), Err(PlanError::NotOwnerOnly));
        std::fs::remove_file(&path).unwrap();
    }
}
