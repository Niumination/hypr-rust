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
    DuplicateWindowId,
    InsufficientSpace,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds => write!(f, "display bounds must be finite and positive"),
            Self::InvalidConfig => write!(f, "layout gaps/ratio are outside valid bounds"),
            Self::DuplicateWindowId => write!(f, "window IDs in one layout must be unique"),
            Self::InsufficientSpace => write!(f, "usable area is too small for requested gaps"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Calculate deterministic window rectangles without making native OS calls.
///
/// Windows retain the order supplied by the caller. Monocle intentionally
/// overlaps all windows; the application/platform adapter must not interpret
/// this as a request to hide or minimize any window.
pub fn compute_layout(
    bounds: Rect,
    windows: &[WindowId],
    kind: LayoutKind,
    config: LayoutConfig,
) -> Result<Vec<Placement>, LayoutError> {
    if !bounds.is_finite_and_positive() {
        return Err(LayoutError::InvalidBounds);
    }
    validate_config(config)?;

    let mut seen = HashSet::with_capacity(windows.len());
    if windows.iter().any(|window| !seen.insert(*window)) {
        return Err(LayoutError::DuplicateWindowId);
    }
    if windows.is_empty() {
        return Ok(Vec::new());
    }

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
}
