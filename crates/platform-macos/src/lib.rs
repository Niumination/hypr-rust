//! macOS platform adapter (M1 vertical slice).
//!
//! Platform-neutral boundary: this crate depends on `hypr-rust-layout-core`
//! for `Rect` and contains all Apple-framework `unsafe` FFI behind a small
//! safe API. See `docs/MACOS-TAHOE-NEXT-STAGE.md` for scope and acceptance.
//!
//! `unsafe` is confined to `ffi.rs` (raw declarations) and `adapter.rs`
//! (documented wrappers); no other module contains unsafe blocks.

mod adapter;
mod ffi;

pub use adapter::{map_ax_error, process_alive, AxError, AxWindow, MacAxAdapter};
