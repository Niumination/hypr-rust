# Hypr-Rust

**Current state (2026-11-04):** documentation + platform-neutral `layout-core` (five layouts — BSP, master/stack, monocle, columns, rows — plus rules/exclusions and clamp-aware geometry; no OS dependencies) + the narrow single-window macOS AX proof (M1 adapter + M2 `ax-keyboard`; host-bound, owner-reported) + a native macOS slice that is **owner-validated on real windows**: strict real-window reflow ([ADR-0006](docs/adr/0006-real-window-reflow-strict-no-partial.md) — preflight read-before-write, verified writes, full rollback, honest three-way status; clamp-skip amendment §2.5 host-verified 2026-11-03) reachable from the menu and the global hotkey `⌥⌘R` ([ADR-0007](docs/adr/0007-global-hotkey-reflow.md)), plus opt-in event-driven auto-tiling ([ADR-0008](docs/adr/0008-auto-tiling-event-driven.md); **off until explicitly enabled**), all driven by an owner-only config file (`~/.hypr-rust/config.json`) with per-app dialog rules and a settings panel with a row-based rules editor. The fixture-bound self-test path keeps best-effort rollback ([ADR-0005](docs/adr/0005-batch-multiwindow-best-effort-rollback.md); never claimed atomic) and never touches user windows. **Not yet a usable desktop app:** the Windows adapter is not implemented, macOS Spaces remain disabled, and packaging is a local unsigned `.app` assembled by [`scripts/package-macos.sh`](scripts/package-macos.sh) (no signing/notarization, no distribution claims — [docs/USING-AX-SHELL.md](docs/USING-AX-SHELL.md)) while beta/release packaging awaits a proven signing path. Every native claim is host-bound (owner's ThinkPad X13 Yoga Gen 1 running macOS Tahoe 26.7.1 — not Apple hardware, not other macOS versions); GitHub CI on `macos-26` runs fmt/clippy/tests plus packaging on every PR but is not runtime validation.

## Project boundary

Hypr-Rust is planned as a Rust tiling window manager/controller. The immediate test host, per owner clarification, is a ThinkPad X13 Yoga Gen 1 (Intel i5-10310U) dual-booting Windows and macOS Tahoe 26.7.1. The current milestone is the macOS Tahoe vertical slice — menu-bar shell, settings/rules UI, strict real-window reflow, global hotkey and opt-in auto-tiling — with operator/production hardening (signing/distribution, permission guidance) as the next stage; Windows native work is deferred until the next Windows boot. This does not establish support for Apple-branded Macs or other macOS versions. The product is not a Wayland compositor port. macOS native workspaces remain disabled; never emulate them by hiding/minimizing windows.

## Repository map

- `crates/layout-core/` — pure Rust geometry and deterministic layout planning; no OS dependencies.
- `crates/platform-macos/` — native macOS slice: AX adapter, menu-bar shell + settings/rules panel, strict real-window reflow, global hotkey, opt-in auto-tiling, fixture self-tests. **Not a workspace member** — build/test it with `--manifest-path crates/platform-macos/Cargo.toml`.
- `docs/` — PRD, architecture, roadmap, quality plan, REA research method and ADRs.
- `docs/CAPABILITY-MATRIX.md` — per-platform capability states with links to the evidence for each claim.
- `docs/USING-AX-SHELL.md` — operator guide: menu, `⌥⌘R` hotkey, auto-tiling opt-in, rules, and honest refusal/partial semantics.
- `docs/adr/` — architecture decision records 0001–0008 (product boundary, adapters, workspaces, batch semantics, strict real-window reflow, hotkey, auto-tiling).
- `docs/PLATFORM-SPIKES.md` — native test protocol and results template for the Windows and Intel Mac machines.
- `docs/AGENT-HANDOFF.md` — branch-by-branch OpenCode/Hermes workflow and platform-specific task prompts.
- `docs/USER-DEVICE-PROFILE.md` — current machine, validation boundary and reconciliations needed.
- `docs/MACOS-TAHOE-NEXT-STAGE.md` — focused M1 Rust AX adapter task, safety gate and agent prompt.
- `docs/SYSTEM-INFO-COLLECTION.md` and `scripts/collect-macos-env.sh` — read-only system details for reproducible reports.
- `AGENTS.md` — instructions for OpenCode, Hermes Agent and other coding agents.

## Build and test

Install the stable Rust toolchain, then run from this directory:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# platform-macos is NOT a workspace member — check it separately:
cargo fmt --manifest-path crates/platform-macos/Cargo.toml --check
cargo clippy --manifest-path crates/platform-macos/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path crates/platform-macos/Cargo.toml
```

Both crates pass `cargo fmt --check`, `cargo clippy -D warnings` and their test suites on the owner's host (macOS Tahoe 26.7.1) and in CI on every PR. See [`docs/ARENA-VERIFICATION.md`](docs/ARENA-VERIFICATION.md) for the exact runs, dates and commit identities, and [`docs/CAPABILITY-MATRIX.md`](docs/CAPABILITY-MATRIX.md) for the current per-capability states and test counts. All claims remain host-bound: no claim is made for Apple-branded Macs, other macOS versions or Windows.

## AI-agent handoff

1. Give the agent the full workspace (folder or ZIP), including `AGENTS.md` and `docs/`.
2. Ask it to inspect the current branch and docs, then create a dedicated branch for one bounded task or one OS spike.
3. For native work, run on the actual target OS; a cross-build is not runtime proof. Record findings using `docs/PLATFORM-SPIKES.md`.
4. Have the agent report commit SHA, exact OS/build, exact commands and pass/fail results, limitations and changed files. Do not include credentials or unredacted user window data.
5. Push to the upstream repo and review as a PR. Keep Windows and macOS spike results separate and evidence-backed.
6. Share the public repo URL/commit or an exported patch with the reviewer. A later review must compare exact commits and test evidence; never infer a native feature works from source changes alone.

## Read first

- [Documentation index](docs/README.md)
- [Capability matrix](docs/CAPABILITY-MATRIX.md)
- [Operator guide — USING-AX-SHELL](docs/USING-AX-SHELL.md)
- [Current device profile](docs/USER-DEVICE-PROFILE.md)
- [macOS Tahoe next stage](docs/MACOS-TAHOE-NEXT-STAGE.md)
- [Read-only system information collection](docs/SYSTEM-INFO-COLLECTION.md)
- [Platform spike plan](docs/PLATFORM-SPIKES.md)
- [Agent handoff plan](docs/AGENT-HANDOFF.md)
- [Agent workflow and project rules](AGENTS.md)
