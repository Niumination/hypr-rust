# Arena.ai Verification Brief — hypr-rust

**Purpose:** let an external verifier (arena.ai) check every public claim in this repo without access to the owner's host. Everything below is either (a) reproducible from the public repo alone, or (b) explicitly marked as host-bound evidence with the reason.

**Repo:** https://github.com/Niumination/hypr-rust (public) · local: `labs/hypr-rust/`  
**Date:** 2026-10-19 (updated after arena.ai round-5 review — C9 softened, C14 lock TOCTOU, honest limits)

---

## 1. Artifact map (all public)

| Branch | Commit | PR | Content |
|---|---|---|---|
| `main` | `b4800c7` | — | Doc bundle; Hackintosh host profile; arena brief |
| `spike/macos-spike-1` | `0eb9a25` | [#1](https://github.com/Niumination/hypr-rust/pull/1) | Swift AX spike (Dock mutation removed), Spaces DISABLED, host-label reconciliation, lifecycle-bounded fixture registration (token + flock, no unused helpers, token not printed) |
| `core/tiling-constraints` | `51cfced` | [#2](https://github.com/Niumination/hypr-rust/pull/2) | Min-size constraints + overflow policy (13 tests) |
| `macos/tahoe-ax-adapter-m1` | `3d3df82` | [#3](https://github.com/Niumination/hypr-rust/pull/3) | M1 Rust AX adapter (rollback on all post-mutation failures + flock-gated registration, EWOULDBLOCK-only lock check); merged to main |
| `macos/m2-keyboard-plan` | `d4aafa0` | — | M2 keyboard-first plan + `ax-keyboard` implementation (select/move/resize/raise) |

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
| C9 | Preflight + best-effort rollback; recovery failure reported (NOT atomic) | report §5/§6/§7; `adapter.rs` `validate_frame_rect` (`Rect::is_finite_and_positive`), `set_window_frame_txn` (PositionWriteFailed / SizeAfterPositionFailed / VerificationReadFailed / VerificationFailed) | `ax-probe set-frame --target <pid> --token <hex> nan 0 100 100` → exit 1, window unchanged; 9 mock unit tests; AX not transactional → rollback best-effort (§7) |
| C10 | Workspace builds without macOS | root `Cargo.toml` members = layout-core only; platform-macos standalone `[workspace]` | `cargo test --workspace` on any OS → layout-core only, no Apple link |
| C11 | (F1) Mutation commands require a token matching the self-owned fixture registration; otherwise refused before any AX call | `ax-probe.rs` `verify_target_registration` (file regular non-symlink 0600 uid-match, pid+token match, pid alive, flock lock held — only EWOULDBLOCK/EAGAIN accepted as 'held', other errors reject), exit 5; `adapter.rs` `process_alive` | `ax-probe set-frame <pid> 1 2 3 4` → exit 5; live run §5 |
| C12 | (F2) `set_window_frame` read-back verification: mismatch → full rollback + typed `VerificationFailed`; all post-mutation failures roll back | `adapter.rs` `frames_match` + `set_window_frame_txn` (PositionWriteFailed / SizeAfterPositionFailed / VerificationReadFailed / VerificationFailed); 9 mock unit tests | unit tests 15/15; live run §5 (all read-backs matched) |
| C13 | (F1b) Fixture registration is lifecycle-bounded; rejects stale records when the lock is free (lock does NOT bind fixture identity to the AX operation — see C14) | `spike-macos.swift` `acquireFixtureLock`/`writeRegistration`/`removeRegistration` (flock held for fixture lifetime, atomic 0600 write, cleanup on terminate); `ax-probe.rs` `verify_fixture_lock` | live run §5: lock held while alive, freed on death → stale record rejected |
| C14 | Lock check is a test guard, not absolute PID-reuse proof (TOCTOU) | `ax-probe.rs` `verify_fixture_lock` (checked before AX, fd closed) | documented: lock proves fixture alive at check time, not at mutation time; stronger claim needs binding fixture identity to the mutation operation (future work) |
| C15 | (M2) Explicit keyboard-first actions on the registered fixture via `ax-keyboard` | `ax-keyboard.rs` `select`/`move`/`resize`/`raise`; `fixture.rs` `verify_registration` | native run §M2: select ok · no-token exit 5 · move/resize/raise ok · invalid exit 1 · stale exit 4 |
| C16 | (M2) No loop / global hotkey / auto-tiling; raise explicit only; failure typed | `ax-keyboard.rs` (one-shot CLI actions, no loop); `AxError` + `FixtureError` typed errors | native run §M2; code has no background loop or hotkey daemon |

## 3. Host-bound items (cannot be verified from arena.ai)

| Item | Status | Why |
|---|---|---|
| Native AX run (C4/C5/C9/C11/C12/C13/C15/C16) | **executed on owner host**, verbatim in report §5 / M2 §Native | requires the physical Hackintosh + AX grant + self-owned target window |
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
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test                      # 15/15
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --bin ax-probe --bin ax-keyboard

# native (owner host): harness from spike branch @ 0eb9a25
git show spike/macos-spike-1:spikes/macos/spike-macos.swift > /tmp/spike-macos.swift
swiftc -O /tmp/spike-macos.swift -o /tmp/spike-macos && /tmp/spike-macos spawn-target &
# capture PID (pgrep) and TOKEN from ~/.hypr-rust/fixture-target.json (0600)
./target/debug/ax-probe frame <pid>                                                # read-only, no token
./target/debug/ax-probe set-frame --target <pid> --token <hex> 120 120 420 312      # mutate
./target/debug/ax-probe raise --target <pid> --token <hex>                          # mutate
./target/debug/ax-probe set-frame <pid> 120 120 420 312                             # exit 5 (no token)
kill <pid>; ./target/debug/ax-probe frame <pid>                                    # exit 4 (stale)

# M2 keyboard-first (explicit one-shot actions on the registered fixture)
./target/debug/ax-keyboard select --target <pid> --token <hex>                      # validate fixture
./target/debug/ax-keyboard move --target <pid> --token <hex> 120 120                # explicit move
./target/debug/ax-keyboard resize --target <pid> --token <hex> 480 320              # explicit resize
./target/debug/ax-keyboard raise --target <pid> --token <hex>                       # explicit raise
./target/debug/ax-keyboard move <pid> 120 120                                       # exit 5 (no token)
```

Note: `MACOSX_DEPLOYMENT_TARGET=26.0` needed only because the owner's rustc (Homebrew 1.98.1) ships a std built for macOS 26; default x86_64 min is 10.12. Not a code issue.

## 5. Honesty constraints honored

- No serials, UUIDs, MLB, credentials, unredacted window titles, or complete EFI configs in the repo (house rule; EFI read was `SystemProductName` only).
- Permission-denied and Windows gaps marked, not hidden.
- Spike #1 host label reconciled (was "MacBook Pro 13-inch class" → Hackintosh on ThinkPad X13 Yoga).
- Arena.ai M1 closeout packet (F1/F1b/F2) + round-4/5 reviews addressed; mapping in report §6. Mock transaction tests labelled as mock; native rollback-denial not runtime-forced. C9 softened (rollback best-effort, not atomic); C14 documents lock TOCTOU as test guard, not absolute PID-reuse proof.
