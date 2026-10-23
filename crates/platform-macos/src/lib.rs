//! macOS platform adapter (M1 vertical slice).
//!
//! Platform-neutral boundary: this crate depends on `hypr-rust-layout-core`
//! for `Rect` and contains all Apple-framework `unsafe` FFI behind a small
//! safe API. See `docs/MACOS-TAHOE-NEXT-STAGE.md` for scope and acceptance.
//!
//! `unsafe` is confined to `ffi.rs` (raw declarations) and `adapter.rs`
//! (documented wrappers); no other module contains unsafe blocks.

pub mod adapter;
pub mod batch;
pub mod ffi;
pub mod fixture;

pub use adapter::{map_ax_error, process_alive, AxError, AxWindow, MacAxAdapter};
pub use batch::{
    apply_batch, parse_plan, verify_plan_file, BatchOps, BatchResult, BatchTarget, PlanError,
    WindowKey, WindowOutcome,
};
pub use fixture::FixtureError;
