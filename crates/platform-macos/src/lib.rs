//! macOS platform adapter (M1 vertical slice).
//!
//! Platform-neutral boundary: this crate depends on `hypr-rust-layout-core`
//! for `Rect` and contains all Apple-framework `unsafe` FFI behind a small
//! safe API. See `docs/MACOS-TAHOE-NEXT-STAGE.md` for scope and acceptance.
//!
//! `unsafe` is confined to `ffi.rs` (raw declarations), `adapter.rs`
//! (documented wrappers), `shell.rs` (Objective-C messaging) and `hotkey.rs`
//! (Carbon hotkey handler); no other module contains unsafe blocks.

pub mod adapter;
pub mod batch;
pub mod config;
pub mod ffi;
pub mod fixture;
pub mod hotkey;
pub mod shell;

pub use adapter::{
    global_window_key, map_ax_error, observation_from_windows, process_alive,
    split_global_window_key, AxError, AxWindow, MacAxAdapter, MacMultiWindowSet, MacObservation,
    MacWindowSet, WindowIdMap, WindowKey,
};
pub use batch::{
    apply_batch, apply_batch_strict, parse_plan, targets_for_plan, verify_plan_file, BatchOps,
    BatchResult, BatchTarget, PlanError, StrictBatchResult, StrictStatus, WindowOutcome,
};
pub use fixture::FixtureError;
