# Arena.ai Verification Brief — hypr-rust

**Purpose:** let an external verifier (arena.ai) check every public claim in this repo without access to the owner's host. Everything below is either (a) reproducible from the public repo alone, or (b) explicitly marked as host-bound evidence with the reason.

**Repo:** https://github.com/Niumination/hypr-rust (public) · local: `labs/hypr-rust/`  
**Date:** 2026-10-18

---

## 1. Artifact map (all public)

| Branch | Commit | PR | Content |
|---|---|---|---|
| `main` | `f505dbe` | — | Doc bundle (M1 stage note, device profile, env collector, ADRs, PRD) |
| `spike/macos-spike-1` | `501c7d3` | [#1](https://github.com/Niumination/hypr-rust/pull/1) | Swift AX spike (Dock mutation removed), Spaces DISABLED, host-label reconciliation |
| `core/tiling-constraints` | `51cfced` | [#2](https://github.com/Niumination/hypr-rust/pull/2) | Min-size constraints + overflow policy (13 tests) |
| `macos/tahoe-ax-adapter-m1` | `c3950d8` | [#3](https://github.com/Niumination/hypr-rust/pull/3) | M1 Rust AX adapter `crates/platform-macos` + verification report |

## 2. Claims → evidence → how to verify

| # | Claim | Evidence (public) | Verify command |
|---|---|---|---|
| C1 | layout-core compiles clean, 6 tests pass | `crates/layout-core`, branch `main` | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` |
| C2 | Constrained planner: min-size clamp + diagnostics, order preserved, deterministic | PR #2 report + 13 tests incl. LCG property test | checkout PR #2 branch, same commands (13 tests) |
| C3 | Swift spike: AX read/move/resize/raise PASS on self-owned window; Spaces NOT AVAILABLE via public API; Dock mutation removed | `docs/PLATFORM-SPIKES.md`, `spikes/macos/spike-macos.swift` (no Dock code) | code search: `grep -ri dock spikes/` → only comments; `grep -n "runTests" spike-macos.swift` shows target-only mutation |
| C4 | Rust adapter discovers/moves/resizes/raises a self-owned window via public AX/CoreGraphics FFI | `docs/reports/MACOS-M1-AX-ADAPTER-2026-10-18.md` §5 (verbatim run) | requires macOS host with AX grant — see §3 |
| C5 | Stale target → typed error, exit 4, prompt, no crash/retry/false-success | report §5; `adapter.rs` (`kill(pid,0)` pre-check + `AxError::StaleTarget`) | `kill <pid>; ax-probe frame <pid>` → exit 4 |
| C6 | No private APIs, no deps beyond workspace, no secrets | `crates/platform-macos/Cargo.toml` (single path dep); `ffi.rs` (only ApplicationServices/CoreFoundation/libSystem public symbols) | `grep -rE "SkyLight|CGSPrivate|SLSC" crates/` → empty; `cargo tree -p hypr-rust-platform-macos` |
| C7 | Core is platform-neutral (no Apple frameworks in shared core) | `crates/layout-core` has zero deps, no `cfg(target_os)` | `cargo tree -p hypr-rust-layout-core` → empty; code search for `core_foundation`/`objc` → empty |
| C8 | Host identity: Hackintosh (OpenCore SMBIOS spoof `MacBookPro16,2`) on ThinkPad X13 Yoga Gen 1 | `docs/USER-DEVICE-PROFILE.md` §"Host identity — confirmed Hackintosh" (evidence table + EFI `SystemProductName` read) | owner-host bound — see §3 |

## 3. Host-bound items (cannot be verified from arena.ai)

| Item | Status | Why |
|---|---|---|
| Native AX run (C4/C5) | **executed on owner host**, verbatim in report §5 | requires the physical Hackintosh + AX grant + self-owned target window |
| Permission-denied path | **NOT RUNTIME TESTED** | code path exists (`connect()` gate → `PermissionDenied`, exit 3); revoking AX trust would break the test environment |
| Hackintosh identity (C8) | verified read-only on owner host (kextstat, `hw.model`, EFI `SystemProductName`) | EFI partition not public; evidence table is the public record |
| Windows 11 behavior | **not verified** | no Windows boot this session; `platform-windows` deferred per `MACOS-TAHOE-NEXT-STAGE.md` |

## 4. Reproduction quick start (public repo, any macOS 26 toolchain)

```
git clone https://github.com/Niumination/hypr-rust && cd hypr-rust
git checkout macos/tahoe-ax-adapter-m1
cargo fmt --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --workspace --all-targets -- -D warnings
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace        # 8/8 pass
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build -p hypr-rust-platform-macos --bin ax-probe
```

Note: `MACOSX_DEPLOYMENT_TARGET=26.0` needed only because the owner's rustc (Homebrew 1.98.1) ships a std built for macOS 26; default x86_64 min is 10.12. Not a code issue.

## 5. Honesty constraints honored

- No serials, UUIDs, MLB, credentials, unredacted window titles, or complete EFI configs in the repo (house rule; EFI read was `SystemProductName` only).
- Permission-denied and Windows gaps marked, not hidden.
- Spike #1 host label reconciled (was "MacBook Pro 13-inch class" → Hackintosh on ThinkPad X13 Yoga).
