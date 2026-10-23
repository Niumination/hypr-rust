//! Platform-neutral geometry and deterministic tiling layout planner.
//!
//! This crate intentionally has no OS APIs. Native window control is a separate,
//! not-yet-implemented platform spike and must be validated on target machines.

#![forbid(unsafe_code)]

mod domain;
mod geometry;
mod layout;

pub use domain::{
    plan_from_port, plan_snapshot, DisplayId, DisplaySnapshot, ExcludedWindow, ExclusionReason,
    LayoutPlan, PlanError, PlatformSnapshot, PlatformSnapshotPort, SnapshotReadError,
    WindowCapabilities, WindowRole, WindowSnapshot,
};
pub use geometry::Rect;
pub use layout::{
    compute_layout, compute_layout_constrained, ConstrainedWindow, LayoutConfig, LayoutDiagnostic,
    LayoutError, LayoutKind, LayoutResult, Placement, WindowConstraints, WindowId,
};
