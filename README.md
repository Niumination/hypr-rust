# Hypr-Rust

**Current state:** documentation + platform-neutral layout-core scaffold + narrow single-window macOS AX control proof (M1 adapter + M2 `ax-keyboard`; host-bound, owner-reported; the read-only 2/3-window enumeration/geometry probe is separate and is not control evidence) + a batch multi-window executor with best-effort rollback ([ADR-0005](docs/adr/0005-batch-multiwindow-best-effort-rollback.md); native-run on the owner's host — two-window success and two-window rollback verified, permission-denied runtime-tested in one AX-untrusted context) + a minimal Rust `NSStatusItem` menu-bar shell driven by an owner-only config file (`~/.hypr-rust/config.json`; auto-tiling off until explicitly enabled). Not yet a usable desktop app: Windows adapter, the settings panel carries a JSON-text rules editor for the ordinary-app dialog policy (same validation path as a file load), hotkeys and packaging remain to be implemented and tested (the menu-bar shell carries a minimal settings panel over the owner-only config file).

## Project boundary

Hypr-Rust is planned as a Rust tiling window manager/controller. The immediate test host, per owner clarification, is a ThinkPad X13 Yoga Gen 1 (Intel i5-10310U) dual-booting Windows and macOS Tahoe 26.7.1. The current milestone focuses on a narrow macOS Accessibility adapter proof on that exact host; Windows native work is deferred until the next Windows boot. This does not establish support for Apple-branded Macs or other macOS versions. The product is not a Wayland compositor port. macOS native workspaces remain disabled; never emulate them by hiding/minimizing windows.

## Repository map

- `crates/layout-core/` — pure Rust geometry and deterministic layout planning; no OS dependencies.
- `docs/` — PRD, architecture, roadmap, quality plan, REA research method and ADRs.
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
```

This workspace is compiled and tested on the owner's host (macOS Tahoe 26.7.1, branch `work/phase1-domain`): `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass there. See [`docs/ARENA-VERIFICATION.md`](docs/ARENA-VERIFICATION.md) for the exact runs, dates and commit identities. All claims remain host-bound: no claim is made for Apple-branded Macs, other macOS versions or Windows.

## AI-agent handoff

1. Give the agent the full workspace (folder or ZIP), including `AGENTS.md` and `docs/`.
2. Ask it to inspect the current branch and docs, then create a dedicated branch for one bounded task or one OS spike.
3. For native work, run on the actual target OS; a cross-build is not runtime proof. Record findings using `docs/PLATFORM-SPIKES.md`.
4. Have the agent report commit SHA, exact OS/build, exact commands and pass/fail results, limitations and changed files. Do not include credentials or unredacted user window data.
5. Push to the upstream repo and review as a PR. Keep Windows and macOS spike results separate and evidence-backed.
6. Share the public repo URL/commit or an exported patch with the reviewer. A later review must compare exact commits and test evidence; never infer a native feature works from source changes alone.

## Read first

- [Documentation index](docs/README.md)
- [Current device profile](docs/USER-DEVICE-PROFILE.md)
- [macOS Tahoe next stage](docs/MACOS-TAHOE-NEXT-STAGE.md)
- [Read-only system information collection](docs/SYSTEM-INFO-COLLECTION.md)
- [Platform spike plan](docs/PLATFORM-SPIKES.md)
- [Agent handoff plan](docs/AGENT-HANDOFF.md)
- [Agent workflow and project rules](AGENTS.md)
