# Arena.ai Verification Brief — hypr-rust

**Purpose:** let an external verifier (arena.ai) check every public claim in this repo without access to the owner's host. Everything below is either (a) reproducible from the public repo alone, or (b) explicitly marked as host-bound evidence with the reason.

**Repo:** https://github.com/Niumination/hypr-rust (public) · local: `labs/hypr-rust/`  
**Date:** 2026-10-18 (updated after arena.ai review of PR #3 — all findings addressed)

---

## 1. Artifact map (all public)

| Branch | Commit | PR | Content |
|---|---|---|---|
| `main` | `b4800c7` | — | Doc bundle; Hackintosh host profile; arena brief |
| `spike/macos-spike-1` | `5500727` | [#1](https://github.com/Niumination/hypr-rust/pull/1) | Swift AX spike (Dock mutation removed), Spaces DISABLED, host-label reconciliation, token registration at `spawn-target` |
| `core/tiling-constraints` | `51cfced` | [#2](https://github.com/Niumination/hypr-rust/pull/2) | Min-size constraints + overflow policy (13 tests) |
| `macos/tahoe-ax-adapter-m1` | `41a8867` | [#3](https://github.com/Niumination/hypr-rust/pull/3) | M1 Rust AX adapter (token-gated mutation + read-back verification + rollback); report/brief in the following docs commit |

Platform-macos is deliberately **outside** workspace members: `cargo build/test --workspace` stays Windows-safe; the macOS crate is built via its own manifest (see §4).

## 2. Claims → evidence → how to verify

| # | Claim | Evidence (public) | Verify command |
|---|---|---|---|
| C1 | layout-core compiles clean, 6 tests pass | `crates/layout-core` | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` |
| C2 | Constrained planner: min-size clamp + diagnostics, order preserved, deterministic | PR #2 + 13 tests incl. LCG property test | checkout PR #2 branch, same commands (13 tests) |
| C3 | Swift spike: AX read/move/resize/raise PASS on self-owned window; Spaces NOT AVAILABLE via public API; Dock mutation removed | `docs/PLATFORM-SPIKES.md`, `spikes/macos/spike-macos.swift` | `grep -ri dock spikes/` → comments only; `grep -n "dock" spikes/macos/spike-macos.swift` → removal note; workspace section documents NOT AVAILABLE |
| C4 | Rust adapter discovers/moves/resizes/raises a self-owned window via public AX/CoreGraphics FFI | `docs/reports/MACOS-M1-AX-ADAPTER-2026-10-18.md` §5 (verbatim run) | requires macOS host with AX grant — see §3 |
| C5 | Stale target → typed error, exit 4, prompt, no crash/retry/false-success | report §5; `adapter.rs` (`kill(pid,0)` + errno ESRCH check, `AxError::StaleTarget`) | `kill <pid>; ax-probe frame <pid>` → exit 4 |
| C6 | No private APIs, no deps beyond workspace path-dep, no secrets | `crates/platform-macos/Cargo.toml` (single path dep); `ffi.rs` (only ApplicationServices/CoreFoundation/libSystem public symbols) | `grep -rE "SkyLight|CGSPrivate|SLSC" crates/` → empty |
| C7 | Core is platform-neutral (no Apple frameworks in shared core) | `crates/layout-core` has zero deps, no `cfg(target_os)` | `cargo tree -p hypr-rust-layout-core` → empty |
| C8 | Host identity: Hackintosh (OpenCore SMBIOS spoof `MacBookPro16,2`) on ThinkPad X13 Yoga Gen 1 | `docs/USER-DEVICE-PROFILE.md` §"Host identity — confirmed Hackintosh" (evidence table + EFI `SystemProductName` read) | owner-host bound — see §3 |
| C9 | No partial mutation: geometry preflight + rollback | report §5/§6; `adapter.rs` `validate_frame_rect` (`Rect::is_finite_and_positive`), `SizeAfterPositionFailed` | `ax-probe set-frame --target <pid> --token <hex> nan 0 100 100` → exit 1, window unchanged; unit test `frame_validation_rejects_non_finite_and_non_positive` |
| C10 | Workspace builds without macOS | root `Cargo.toml` members = layout-core only; platform-macos standalone `[workspace]` | `cargo test --workspace` on any OS → layout-core only, no Apple link |
| C11 | (F1) Mutation commands require a token matching the self-owned fixture registration; otherwise refused before any AX call | `ax-probe.rs` `verify_target_registration` (file 0600, pid+token match, pid alive), exit 5; `adapter.rs` `process_alive` | `ax-probe set-frame <pid> 1 2 3 4` → exit 5; unit tests + live run §5 |
| C12 | (F2) `set_window_frame` read-back verification: mismatch → full rollback + typed `VerificationFailed` | `adapter.rs` `frames_match` + read-back in `set_window_frame`; unit test `frames_match_uses_tolerance` | unit test 6/6; live run §5 (all read-backs matched) |

## 3. Host-bound items (cannot be verified from arena.ai)

| Item | Status | Why |
|---|---|---|
| Native AX run (C4/C5/C9/C11/C12) | **executed on owner host**, verbatim in report §5 | requires the physical Hackintosh + AX grant + self-owned target window |
| Permission-denied path | **NOT RUNTIME TESTED** | code path exists (`connect()` gate → `PermissionDenied`, exit 3); revoking AX trust would break the test environment |
| Hackintosh identity (C8) | verified read-only on owner host (kextstat, `hw.model`, EFI `SystemProductName`) | EFI partition not public; evidence table is the public record |
| Windows 11 behavior | **not verified** | no Windows boot this session; `platform-windows` deferred per `MACOS-TAHOE-NEXT-STAGE.md` |

## 4. Reproduction quick start (public repo, any macOS 26 toolchain)

```
git clone https://github.com/Niumination/hypr-rust && cd hypr-rust
git checkout macos/tahoe-ax-adapter-m1

# workspace (Windows-safe)
cargo fmt --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --workspace --all-targets -- -D warnings
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --workspace          # 6/6

# macOS-only crate (standalone)
cargo fmt --manifest-path crates/platform-macos/Cargo.toml --check
MACOSX_DEPLOYMENT_TARGET=26.0 cargo clippy --manifest-path crates/platform-macos/Cargo.toml --all-targets -- -D warnings
cd crates/platform-macos
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test                      # 6/6
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --bin ax-probe

# native (owner host): harness from spike branch @ 5500727
git show spike/macos-spike-1:spikes/macos/spike-macos.swift > /tmp/spike-macos.swift
swiftc -O /tmp/spike-macos.swift -o /tmp/spike-macos && /tmp/spike-macos spawn-target &
# capture PID (pgrep) and TOKEN from /tmp/hypr-rust-target.json (perm 0600)
./target/debug/ax-probe frame <pid>                                                # read-only, no token
./target/debug/ax-probe set-frame --target <pid> --token <hex> 120 120 420 312      # mutate
./target/debug/ax-probe raise --target <pid> --token <hex>                          # mutate
./target/debug/ax-probe set-frame <pid> 120 120 420 312                             # exit 5 (no token)
kill <pid>; ./target/debug/ax-probe frame <pid>                                    # exit 4 (stale)
```

Note: `MACOSX_DEPLOYMENT_TARGET=26.0` needed only because the owner's rustc (Homebrew 1.98.1) ships a std built for macOS 26; default x86_64 min is 10.12. Not a code issue.

## 5. Honesty constraints honored

- No serials, UUIDs, MLB, credentials, unredacted window titles, or complete EFI configs in the repo (house rule; EFI read was `SystemProductName` only).
- Permission-denied and Windows gaps marked, not hidden.
- Spike #1 host label reconciled (was "MacBook Pro 13-inch class" → Hackintosh on ThinkPad X13 Yoga).
- Arena.ai review findings all addressed; mapping in report §6.
