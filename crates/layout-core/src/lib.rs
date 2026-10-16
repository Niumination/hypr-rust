//! Platform-neutral geometry and deterministic tiling layout planner.
//!
//! This crate intentionally has no OS APIs. Native window control is a separate,
//! not-yet-implemented platform spike and must be validated on target machines.

#![forbid(unsafe_code)]

mod geometry;
mod layout;

pub use geometry::Rect;
pub use layout::{compute_layout, LayoutConfig, LayoutError, LayoutKind, Placement, WindowId};
