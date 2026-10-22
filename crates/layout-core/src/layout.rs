use crate::Rect;
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    /// Recursively bisect the available rectangle, alternating split axes.
    Bsp,
    /// One primary window on the left; remaining windows form a vertical stack.
    MasterStack,
    /// All windows share the same rectangle; the OS/application z-order decides visibility.
    Monocle,
    /// One full-height column per window.
    Columns,
    /// One full-width row per window.
    Rows,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutConfig {
    pub outer_gap: f64,
    pub inner_gap: f64,
    pub master_ratio: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            outer_gap: 12.0,
            inner_gap: 8.0,
            master_ratio: 0.55,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub window: WindowId,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    InvalidBounds,
    InvalidConfig,
    InvalidConstraints,
    DuplicateWindowId,
    InsufficientSpace,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds => write!(f, "display bounds must be finite and positive"),
            Self::InvalidConfig => write!(f, "layout gaps/ratio are outside valid bounds"),
            Self::InvalidConstraints => {
                write!(f, "window minimum sizes must be finite and non-negative")
            }
            Self::DuplicateWindowId => write!(f, "window IDs in one layout must be unique"),
            Self::InsufficientSpace => write!(f, "usable area is too small for requested gaps"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Calculate deterministic window rectangles without making native OS calls.
///
/// This is the legacy unconstrained entry point: it delegates to
/// [`compute_layout_constrained`] with [`WindowConstraints::UNBOUNDED`] and
/// therefore behaves identically to the pre-constraints planner.
///
/// Windows retain the order supplied by the caller. Monocle intentionally
/// overlaps all windows; the application/platform adapter must not interpret
/// this as a request to hide or minimize any window.
///
/// # Rounding policy
///
/// The planner works in exact `f64` logical coordinates and never truncates
/// inside the planner. Platform adapters round to device pixels at the OS
/// boundary; identical inputs produce bit-identical plans.
pub fn compute_layout(
    bounds: Rect,
    windows: &[WindowId],
    kind: LayoutKind,
    config: LayoutConfig,
) -> Result<Vec<Placement>, LayoutError> {
    let constrained: Vec<ConstrainedWindow> = windows
        .iter()
        .map(|id| ConstrainedWindow::unbounded(*id))
        .collect();
    Ok(compute_layout_constrained(bounds, &constrained, kind, config)?.placements)
}
/// Minimum-size constraints for one managed window, in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowConstraints {
    pub min_width: f64,
    pub min_height: f64,
}

impl WindowConstraints {
    /// Unbounded: never clamps; preserves the legacy `compute_layout` behavior.
    pub const UNBOUNDED: Self = Self {
        min_width: 0.0,
        min_height: 0.0,
    };
}

impl Default for WindowConstraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

/// A window together with its minimum-size constraints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConstrainedWindow {
    pub id: WindowId,
    pub constraints: WindowConstraints,
}

impl ConstrainedWindow {
    pub fn unbounded(id: WindowId) -> Self {
        Self {
            id,
            constraints: WindowConstraints::UNBOUNDED,
        }
    }
}

/// Post-placement diagnostics. Diagnostics never fail a plan; they report
/// decisions the engine made on the window's behalf so the engine/executor
/// can surface them instead of silently diverging from the computed plan.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutDiagnostic {
    /// The computed rectangle was smaller than the window minimum and was
    /// clamped up to it (shifted left/up when needed to stay in bounds).
    MinSizeClamped {
        window: WindowId,
        min_w: f64,
        min_h: f64,
        emitted_w: f64,
        emitted_h: f64,
    },
    /// The window minimum itself exceeds the available area; the emitted
    /// rectangle was capped to the available area.
    MinSizeExceedsBounds {
        window: WindowId,
        min_w: f64,
        min_h: f64,
        available_w: f64,
        available_h: f64,
    },
}

/// Complete result of a constrained planning pass: the emitted plan plus
/// any per-window diagnostics produced by the overflow policy.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutResult {
    pub placements: Vec<Placement>,
    pub diagnostics: Vec<LayoutDiagnostic>,
}

/// Deterministic layout planning with per-window minimum-size constraints.
///
/// [`compute_layout`] is the unbounded variant of this function.
///
/// # Validation
///
/// Input validation (bounds finiteness/positivity, config bounds, per-window
/// minimum sizes finite and non-negative, duplicate window IDs) happens here,
/// before any geometry is computed; the public entry points are the only
/// validation boundary.
///
/// # Overflow policy (documented, see ARCHITECTURE §6)
///
/// When a computed rectangle is smaller than a window's minimum, the emitted
/// rectangle is clamped up to the minimum and kept inside the work area by
/// shifting it left/up. If the minimum itself exceeds the available area,
/// the rectangle is capped to the available area. Clamping never produces
/// negative dimensions and never hides or minimizes a window. A clamped
/// rectangle may overlap a neighbor's cell — the executor is responsible for
/// resolving such overlaps (e.g. by floating the window) using the reported
/// diagnostics.
pub fn compute_layout_constrained(
    bounds: Rect,
    windows: &[ConstrainedWindow],
    kind: LayoutKind,
    config: LayoutConfig,
) -> Result<LayoutResult, LayoutError> {
    if !bounds.is_finite_and_positive() {
        return Err(LayoutError::InvalidBounds);
    }
    validate_config(config)?;
    let mut seen = HashSet::with_capacity(windows.len());
    if windows.iter().any(|window| !seen.insert(window.id)) {
        return Err(LayoutError::DuplicateWindowId);
    }
    for window in windows {
        validate_constraints(window.constraints)?;
    }
    if windows.is_empty() {
        return Ok(LayoutResult {
            placements: Vec::new(),
            diagnostics: Vec::new(),
        });
    }
    let ids: Vec<WindowId> = windows.iter().map(|window| window.id).collect();
    let base = plan_unchecked(bounds, &ids, kind, config)?;
    let work_area = inset(bounds, config.outer_gap)?;
    // `plan_unchecked` preserves input order (covered by
    // `placements_follow_input_order_for_every_kind`), so placements and
    // windows stay in lockstep here.
    let mut placements = Vec::with_capacity(base.len());
    let mut diagnostics = Vec::new();
    for (placement, window) in base.into_iter().zip(windows) {
        let (rect, diagnostic) =
            clamp_to_minimum(window.id, placement.rect, window.constraints, work_area);
        if let Some(diagnostic) = diagnostic {
            diagnostics.push(diagnostic);
        }
        placements.push(Placement {
            window: window.id,
            rect,
        });
    }
    Ok(LayoutResult {
        placements,
        diagnostics,
    })
}

/// Pure geometry planning without input validation. Callers must validate
/// bounds, config and duplicate IDs first (see [`compute_layout_constrained`]).
fn plan_unchecked(
    bounds: Rect,
    windows: &[WindowId],
    kind: LayoutKind,
    config: LayoutConfig,
) -> Result<Vec<Placement>, LayoutError> {
    let work_area = inset(bounds, config.outer_gap)?;
    let mut result = Vec::with_capacity(windows.len());

    match kind {
        LayoutKind::Bsp => bsp(work_area, windows, 0, config.inner_gap, &mut result)?,
        LayoutKind::MasterStack => {
            master_stack(work_area, windows, config, &mut result)?;
        }
        LayoutKind::Monocle => {
            result.extend(windows.iter().copied().map(|window| Placement {
                window,
                rect: work_area,
            }));
        }
        LayoutKind::Columns => {
            let rects = partition(work_area, windows.len(), config.inner_gap, Axis::X)?;
            result.extend(
                windows
                    .iter()
                    .copied()
                    .zip(rects)
                    .map(|(window, rect)| Placement { window, rect }),
            );
        }
        LayoutKind::Rows => {
            let rects = partition(work_area, windows.len(), config.inner_gap, Axis::Y)?;
            result.extend(
                windows
                    .iter()
                    .copied()
                    .zip(rects)
                    .map(|(window, rect)| Placement { window, rect }),
            );
        }
    }

    debug_assert_eq!(result.len(), windows.len());
    Ok(result)
}

/// Clamp one emitted rectangle up to the window minimum, keeping it inside
/// the work area. Grows from the top-left; shifts left/up when the grown
/// rectangle would cross the work-area edge.
fn clamp_to_minimum(
    window: WindowId,
    rect: Rect,
    constraints: WindowConstraints,
    work_area: Rect,
) -> (Rect, Option<LayoutDiagnostic>) {
    // Constraints are validated by the public entry point (finite, non-negative).
    let min_w = constraints.min_width;
    let min_h = constraints.min_height;
    if rect.width >= min_w && rect.height >= min_h {
        return (rect, None);
    }
    let target_w = rect.width.max(min_w);
    let target_h = rect.height.max(min_h);
    let fits = target_w <= work_area.width && target_h <= work_area.height;
    let w = target_w.min(work_area.width);
    let h = target_h.min(work_area.height);
    let x = rect.x.min(work_area.right() - w).max(work_area.x);
    let y = rect.y.min(work_area.bottom() - h).max(work_area.y);
    let clamped = Rect::new(x, y, w, h);
    let diagnostic = if fits {
        LayoutDiagnostic::MinSizeClamped {
            window,
            min_w,
            min_h,
            emitted_w: w,
            emitted_h: h,
        }
    } else {
        LayoutDiagnostic::MinSizeExceedsBounds {
            window,
            min_w,
            min_h,
            available_w: work_area.width,
            available_h: work_area.height,
        }
    };
    (clamped, Some(diagnostic))
}

fn validate_constraints(constraints: WindowConstraints) -> Result<(), LayoutError> {
    if !constraints.min_width.is_finite()
        || !constraints.min_height.is_finite()
        || constraints.min_width < 0.0
        || constraints.min_height < 0.0
    {
        return Err(LayoutError::InvalidConstraints);
    }
    Ok(())
}

fn validate_config(config: LayoutConfig) -> Result<(), LayoutError> {
    if !config.outer_gap.is_finite()
        || !config.inner_gap.is_finite()
        || !config.master_ratio.is_finite()
        || config.outer_gap < 0.0
        || config.inner_gap < 0.0
        || !(0.05..=0.95).contains(&config.master_ratio)
    {
        return Err(LayoutError::InvalidConfig);
    }
    Ok(())
}

fn inset(bounds: Rect, gap: f64) -> Result<Rect, LayoutError> {
    let width = bounds.width - 2.0 * gap;
    let height = bounds.height - 2.0 * gap;
    if width <= 0.0 || height <= 0.0 {
        return Err(LayoutError::InsufficientSpace);
    }
    Ok(Rect::new(bounds.x + gap, bounds.y + gap, width, height))
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

fn partition(rect: Rect, count: usize, gap: f64, axis: Axis) -> Result<Vec<Rect>, LayoutError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let (origin, length) = match axis {
        Axis::X => (rect.x, rect.width),
        Axis::Y => (rect.y, rect.height),
    };
    let total_gaps = gap * count.saturating_sub(1) as f64;
    let available = length - total_gaps;
    if !available.is_finite() || available <= 0.0 {
        return Err(LayoutError::InsufficientSpace);
    }

    let each = available / count as f64;
    let mut cells = Vec::with_capacity(count);
    for index in 0..count {
        let start = origin + index as f64 * (each + gap);
        let cell_length = if index + 1 == count {
            (origin + length - start).max(0.0)
        } else {
            each
        };
        let cell = match axis {
            Axis::X => Rect::new(start, rect.y, cell_length, rect.height),
            Axis::Y => Rect::new(rect.x, start, rect.width, cell_length),
        };
        if !cell.is_finite_and_positive() {
            return Err(LayoutError::InsufficientSpace);
        }
        cells.push(cell);
    }
    Ok(cells)
}

fn master_stack(
    area: Rect,
    windows: &[WindowId],
    config: LayoutConfig,
    result: &mut Vec<Placement>,
) -> Result<(), LayoutError> {
    if windows.len() == 1 {
        result.push(Placement {
            window: windows[0],
            rect: area,
        });
        return Ok(());
    }

    let available_width = area.width - config.inner_gap;
    let master_width = available_width * config.master_ratio;
    let stack_width = available_width - master_width;
    if master_width <= 0.0 || stack_width <= 0.0 {
        return Err(LayoutError::InsufficientSpace);
    }

    let master_rect = Rect::new(area.x, area.y, master_width, area.height);
    let stack_rect = Rect::new(
        area.x + master_width + config.inner_gap,
        area.y,
        stack_width,
        area.height,
    );
    result.push(Placement {
        window: windows[0],
        rect: master_rect,
    });

    let stack_rows = partition(stack_rect, windows.len() - 1, config.inner_gap, Axis::Y)?;
    result.extend(
        windows[1..]
            .iter()
            .copied()
            .zip(stack_rows)
            .map(|(window, rect)| Placement { window, rect }),
    );
    Ok(())
}

fn bsp(
    area: Rect,
    windows: &[WindowId],
    depth: usize,
    gap: f64,
    result: &mut Vec<Placement>,
) -> Result<(), LayoutError> {
    if windows.is_empty() {
        return Ok(());
    }
    if windows.len() == 1 {
        result.push(Placement {
            window: windows[0],
            rect: area,
        });
        return Ok(());
    }

    let first_count = windows.len().div_ceil(2);
    let ratio = first_count as f64 / windows.len() as f64;
    let axis = if depth.is_multiple_of(2) {
        Axis::X
    } else {
        Axis::Y
    };
    let (first, second) = split(area, gap, ratio, axis)?;

    bsp(first, &windows[..first_count], depth + 1, gap, result)?;
    bsp(second, &windows[first_count..], depth + 1, gap, result)
}

fn split(area: Rect, gap: f64, ratio: f64, axis: Axis) -> Result<(Rect, Rect), LayoutError> {
    match axis {
        Axis::X => {
            let available = area.width - gap;
            let first_width = available * ratio;
            let second_width = available - first_width;
            if first_width <= 0.0 || second_width <= 0.0 {
                return Err(LayoutError::InsufficientSpace);
            }
            Ok((
                Rect::new(area.x, area.y, first_width, area.height),
                Rect::new(
                    area.x + first_width + gap,
                    area.y,
                    second_width,
                    area.height,
                ),
            ))
        }
        Axis::Y => {
            let available = area.height - gap;
            let first_height = available * ratio;
            let second_height = available - first_height;
            if first_height <= 0.0 || second_height <= 0.0 {
                return Err(LayoutError::InsufficientSpace);
            }
            Ok((
                Rect::new(area.x, area.y, area.width, first_height),
                Rect::new(
                    area.x,
                    area.y + first_height + gap,
                    area.width,
                    second_height,
                ),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(count: u64) -> Vec<WindowId> {
        (0..count).map(WindowId).collect()
    }

    fn test_config() -> LayoutConfig {
        LayoutConfig {
            outer_gap: 10.0,
            inner_gap: 6.0,
            master_ratio: 0.6,
        }
    }

    #[test]
    fn all_layouts_return_one_placement_per_window() {
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let ids = windows(7);
        for kind in [
            LayoutKind::Bsp,
            LayoutKind::MasterStack,
            LayoutKind::Monocle,
            LayoutKind::Columns,
            LayoutKind::Rows,
        ] {
            let placements = compute_layout(bounds, &ids, kind, test_config()).unwrap();
            assert_eq!(placements.len(), ids.len());
            for placement in placements {
                assert!(placement.rect.is_finite_and_positive());
                assert!(bounds.contains(placement.rect, 1e-9));
            }
        }
    }

    #[test]
    fn tiled_layouts_do_not_overlap_and_monocle_is_explicitly_overlapping() {
        let bounds = Rect::new(-1920.0, 0.0, 1920.0, 1080.0);
        let ids = windows(6);
        for kind in [
            LayoutKind::Bsp,
            LayoutKind::MasterStack,
            LayoutKind::Columns,
            LayoutKind::Rows,
        ] {
            let placements = compute_layout(bounds, &ids, kind, test_config()).unwrap();
            for left in 0..placements.len() {
                for right in left + 1..placements.len() {
                    assert!(
                        placements[left]
                            .rect
                            .intersection_area(placements[right].rect)
                            < 1e-8
                    );
                }
            }
        }
        let placements = compute_layout(bounds, &ids, LayoutKind::Monocle, test_config()).unwrap();
        assert!(placements
            .windows(2)
            .all(|pair| pair[0].rect == pair[1].rect));
    }

    #[test]
    fn repeated_plans_are_deterministic() {
        let args = (
            Rect::new(0.0, 0.0, 1600.0, 900.0),
            windows(9),
            LayoutKind::Bsp,
            test_config(),
        );
        assert_eq!(
            compute_layout(args.0, &args.1, args.2, args.3),
            compute_layout(args.0, &args.1, args.2, args.3)
        );
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_bounds() {
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(
            compute_layout(
                bounds,
                &[WindowId(1), WindowId(1)],
                LayoutKind::Rows,
                test_config()
            ),
            Err(LayoutError::DuplicateWindowId)
        );
        assert_eq!(
            compute_layout(
                Rect::new(0.0, 0.0, f64::NAN, 600.0),
                &[],
                LayoutKind::Rows,
                test_config()
            ),
            Err(LayoutError::InvalidBounds)
        );
    }

    #[test]
    fn rejects_gaps_that_leave_no_usable_area() {
        let config = LayoutConfig {
            outer_gap: 400.0,
            ..test_config()
        };
        assert_eq!(
            compute_layout(
                Rect::new(0.0, 0.0, 800.0, 600.0),
                &windows(1),
                LayoutKind::Bsp,
                config
            ),
            Err(LayoutError::InsufficientSpace)
        );
    }

    #[test]
    fn empty_window_list_has_empty_plan() {
        let plan = compute_layout(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            &[],
            LayoutKind::Bsp,
            test_config(),
        )
        .unwrap();
        assert!(plan.is_empty());
    }

    fn constrained(count: u64) -> Vec<ConstrainedWindow> {
        (0..count)
            .map(|i| ConstrainedWindow::unbounded(WindowId(i)))
            .collect()
    }

    #[test]
    fn legacy_compute_layout_delegates_to_unconstrained_engine() {
        let bounds = Rect::new(0.0, 0.0, 1600.0, 900.0);
        let ids = windows(9);
        let via_legacy = compute_layout(bounds, &ids, LayoutKind::Bsp, test_config()).unwrap();
        let via_constrained =
            compute_layout_constrained(bounds, &constrained(9), LayoutKind::Bsp, test_config())
                .unwrap();
        assert_eq!(via_legacy, via_constrained.placements);
        assert!(via_constrained.diagnostics.is_empty());
    }

    #[test]
    fn placements_follow_input_order_for_every_kind() {
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let ids = windows(9);
        for kind in [
            LayoutKind::Bsp,
            LayoutKind::MasterStack,
            LayoutKind::Monocle,
            LayoutKind::Columns,
            LayoutKind::Rows,
        ] {
            let plan = compute_layout(bounds, &ids, kind, test_config()).unwrap();
            assert_eq!(plan.len(), ids.len());
            assert!(plan.iter().enumerate().all(|(i, p)| p.window == ids[i]));
        }
    }

    /// Hand-computed golden geometry (QUALITY.md §2): 1920×1080, outer_gap 10,
    /// inner_gap 6 → work area (10,10,1900,1060). Two-way splits with an even
    /// ratio give exactly 947-wide / 527-high cells at these coordinates.
    #[test]
    fn golden_two_window_splits_are_exact() {
        let config = LayoutConfig {
            outer_gap: 10.0,
            inner_gap: 6.0,
            master_ratio: 0.5,
        };
        let expected = vec![
            Placement {
                window: WindowId(0),
                rect: Rect::new(10.0, 10.0, 947.0, 1060.0),
            },
            Placement {
                window: WindowId(1),
                rect: Rect::new(963.0, 10.0, 947.0, 1060.0),
            },
        ];
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        for kind in [
            LayoutKind::Bsp,
            LayoutKind::MasterStack,
            LayoutKind::Columns,
        ] {
            let plan = compute_layout(bounds, &windows(2), kind, config).unwrap();
            assert_eq!(plan, expected, "golden mismatch for {kind:?}");
        }
        let rows = compute_layout(bounds, &windows(2), LayoutKind::Rows, config).unwrap();
        assert_eq!(
            rows,
            vec![
                Placement {
                    window: WindowId(0),
                    rect: Rect::new(10.0, 10.0, 1900.0, 527.0),
                },
                Placement {
                    window: WindowId(1),
                    rect: Rect::new(10.0, 543.0, 1900.0, 527.0),
                },
            ]
        );
    }

    #[test]
    fn golden_monocle_places_every_window_on_the_full_work_area() {
        let plan = compute_layout(
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            &windows(3),
            LayoutKind::Monocle,
            LayoutConfig {
                outer_gap: 10.0,
                inner_gap: 6.0,
                master_ratio: 0.5,
            },
        )
        .unwrap();
        for placement in plan {
            assert_eq!(placement.rect, Rect::new(10.0, 10.0, 1900.0, 1060.0));
        }
    }

    #[test]
    fn minimum_size_clamps_small_computed_rect_with_diagnostic() {
        // Columns×3 on 1920×1080, test_config (outer 10, inner 6) → each
        // cell 629.33 wide. Window 0 demands min 700 wide → clamped to 700
        // at x=10 (top-left kept, still inside the work area).
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let mut input = constrained(3);
        input[0].constraints = WindowConstraints {
            min_width: 700.0,
            min_height: 0.0,
        };
        let result =
            compute_layout_constrained(bounds, &input, LayoutKind::Columns, test_config()).unwrap();
        assert_eq!(
            result.placements[0].rect,
            Rect::new(10.0, 10.0, 700.0, 1060.0)
        );
        assert_eq!(
            result.diagnostics,
            vec![LayoutDiagnostic::MinSizeClamped {
                window: WindowId(0),
                min_w: 700.0,
                min_h: 0.0,
                emitted_w: 700.0,
                emitted_h: 1060.0,
            }]
        );
    }

    #[test]
    fn minimum_size_exceeding_bounds_is_capped_with_diagnostic() {
        // Work area is (10,10,380,280); a 500-wide minimum cannot fit → capped.
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
        let input = vec![ConstrainedWindow {
            id: WindowId(0),
            constraints: WindowConstraints {
                min_width: 500.0,
                min_height: 0.0,
            },
        }];
        let result =
            compute_layout_constrained(bounds, &input, LayoutKind::Monocle, test_config()).unwrap();
        assert_eq!(
            result.placements[0].rect,
            Rect::new(10.0, 10.0, 380.0, 280.0)
        );
        assert_eq!(
            result.diagnostics,
            vec![LayoutDiagnostic::MinSizeExceedsBounds {
                window: WindowId(0),
                min_w: 500.0,
                min_h: 0.0,
                available_w: 380.0,
                available_h: 280.0,
            }]
        );
    }

    #[test]
    fn rejects_invalid_minimum_size_constraints() {
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let invalid = [
            WindowConstraints {
                min_width: -1.0,
                min_height: 0.0,
            },
            WindowConstraints {
                min_width: 0.0,
                min_height: -5.0,
            },
            WindowConstraints {
                min_width: f64::NAN,
                min_height: 0.0,
            },
            WindowConstraints {
                min_width: 0.0,
                min_height: f64::NAN,
            },
            WindowConstraints {
                min_width: f64::INFINITY,
                min_height: 0.0,
            },
            WindowConstraints {
                min_width: 0.0,
                min_height: f64::NEG_INFINITY,
            },
        ];
        for constraints in invalid {
            let input = vec![ConstrainedWindow {
                id: WindowId(0),
                constraints,
            }];
            assert_eq!(
                compute_layout_constrained(bounds, &input, LayoutKind::Monocle, test_config()),
                Err(LayoutError::InvalidConstraints)
            );
        }
    }

    /// xorshift64-style deterministic PRNG (std-only; no test deps).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// Property-style invariants (QUALITY.md §2) across randomized window
    /// counts, display sizes, gaps, ratios, kinds and constraints. Deterministic
    /// by fixed seed; run locally by CI exactly as-is.
    #[test]
    fn randomized_invariants_hold_across_kinds_and_constraints() {
        let kinds = [
            LayoutKind::Bsp,
            LayoutKind::MasterStack,
            LayoutKind::Monocle,
            LayoutKind::Columns,
            LayoutKind::Rows,
        ];
        let mut rng = Lcg(0x5EED_1234_ABCD_0001);
        let bounds = |r: &mut Lcg| {
            Rect::new(
                0.0,
                0.0,
                320.0 + (r.next() % 2000) as f64,
                240.0 + (r.next() % 1400) as f64,
            )
        };
        for _ in 0..400 {
            let b = bounds(&mut rng);
            let n = 1 + rng.next() % 12;
            let config = LayoutConfig {
                outer_gap: (rng.next() % 64) as f64,
                inner_gap: (rng.next() % 32) as f64,
                master_ratio: 0.05 + (rng.next() % 90) as f64 / 100.0,
            };
            let mut input: Vec<ConstrainedWindow> = (0..n)
                .map(|i| ConstrainedWindow {
                    id: WindowId(i),
                    constraints: WindowConstraints {
                        min_width: (rng.next() % 400) as f64,
                        min_height: (rng.next() % 300) as f64,
                    },
                })
                .collect();
            for kind in kinds {
                // InsufficientSpace is a typed, documented error path (covered
                // by unit tests); property invariants apply to successful plans.
                let Ok(result) = compute_layout_constrained(b, &input, kind, config) else {
                    continue;
                };
                assert_eq!(result.placements.len(), n as usize);
                for (i, placement) in result.placements.iter().enumerate() {
                    assert_eq!(placement.window, WindowId(i as u64));
                    assert!(placement.rect.is_finite_and_positive());
                    assert!(b.contains(placement.rect, 1e-9));
                    // Containment holds even for clamped rects because the cap
                    // is the work area itself.
                }
                // No-overlap holds for the unclamped plan. Clamping may grow a
                // rect into a neighbor (documented overflow policy: clamp +
                // report), so the overlap invariant is only asserted when no
                // clamping occurred.
                if kind != LayoutKind::Monocle && result.diagnostics.is_empty() {
                    for left in 0..result.placements.len() {
                        for right in left + 1..result.placements.len() {
                            assert!(
                                result.placements[left]
                                    .rect
                                    .intersection_area(result.placements[right].rect)
                                    < 1e-6,
                                "overlap for {kind:?}"
                            );
                        }
                    }
                }
                // Clamped windows either meet their minimum or are reported
                // via a diagnostic (clamp + report policy).
                for window in &input {
                    let placement = result
                        .placements
                        .iter()
                        .find(|p| p.window == window.id)
                        .unwrap();
                    let meets_min = placement.rect.width >= window.constraints.min_width
                        && placement.rect.height >= window.constraints.min_height;
                    let reported = result.diagnostics.iter().any(|d| match d {
                        LayoutDiagnostic::MinSizeClamped { window: w, .. } => *w == window.id,
                        LayoutDiagnostic::MinSizeExceedsBounds { window: w, .. } => *w == window.id,
                    });
                    assert!(
                        meets_min || reported,
                        "unreported shortfall for window {:?}",
                        window.id
                    );
                }
                // Determinism: same inputs → bit-identical output.
                let again = compute_layout_constrained(b, &input, kind, config).unwrap();
                assert_eq!(result, again);
            }
            let _ = &mut input;
        }
    }
}
