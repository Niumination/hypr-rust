# Hypr-Rust

**Current state:** documentation + first platform-neutral layout-core scaffold. This is not yet a usable desktop app: native adapters, tray/menu-bar UI, settings UI, hotkeys and platform probes remain to be implemented and tested.

## Project boundary

Hypr-Rust is planned as a Rust tiling window manager/controller for Windows 11 x64 and Intel macOS Ventura 13+. It is not a Wayland compositor port. Native workspaces are conditional: the required support set is detecting active/target desktop, switching, moving the focused window, and observing/reporting changes. Never emulate workspace behavior by hiding/minimizing windows.

## Repository map

- `crates/layout-core/` — pure Rust geometry and deterministic layout planning; no OS dependencies.
- `docs/` — PRD, architecture, roadmap, quality plan, REA research method and ADRs.
- `docs/PLATFORM-SPIKES.md` — native test protocol and results template for the Windows and Intel Mac machines.
- `docs/AGENT-HANDOFF.md` — branch-by-branch OpenCode/Hermes workflow and platform-specific task prompts.
- `AGENTS.md` — instructions for OpenCode, Hermes Agent and other coding agents.

## Build and test

Install the stable Rust toolchain, then run from this directory:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

This repository snapshot was authored in an environment without `rustc`/`cargo`; these commands have **not** been run here. Do not treat the layout scaffold as compiled or verified until CI or a developer machine records successful results.

## AI-agent handoff

1. Give the agent the full workspace (folder or ZIP), including `AGENTS.md` and `docs/`.
2. Ask it to inspect the current branch and docs, then create a dedicated branch for one bounded task or one OS spike.
3. For native work, run on the actual target OS; a cross-build is not runtime proof. Record findings using `docs/PLATFORM-SPIKES.md`.
4. Have the agent report commit SHA, exact OS/build, exact commands and pass/fail results, limitations and changed files. Do not include credentials or unredacted user window data.
5. Push to the upstream repo and review as a PR. Keep Windows and macOS spike results separate and evidence-backed.
6. Share the public repo URL/commit or an exported patch with the reviewer. A later review must compare exact commits and test evidence; never infer a native feature works from source changes alone.

## Read first

- [Documentation index](docs/README.md)
- [Product requirements](docs/PRD.md)
- [Platform spike plan](docs/PLATFORM-SPIKES.md)
- [Agent workflow and project rules](AGENTS.md)
