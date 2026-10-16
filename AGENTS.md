# Instructions for AI agents working in this repository

These rules apply to OpenCode, Hermes Agent, and other coding agents. Read `docs/README.md`, `docs/PRD.md`, and the relevant ADR before changing code.

## Source of truth

- Product boundary: a Rust-native tiling window manager/controller, **not** a Wayland compositor port.
- Targets: Windows 11 x64 and Intel macOS Ventura 13+.
- Auto-tiling is off until the user explicitly enables it.
- Native workspaces mean OS Virtual Desktops/Spaces only. The required set is: detect active/target desktop, switch desktop, move the focused window, observe/report. Enable only on an OS/build where all are verified through public APIs. **Never** simulate with minimize/hide, use private APIs, or claim unsupported behavior.
- Quit leaves windows at their last OS-accepted position. Mandatory app-self/system/unsupported-operation exclusions cannot be overridden.

## Required workflow

1. Inspect current code and relevant docs before editing; describe the existing flow and risks.
2. State a short implementation plan and acceptance criteria before making a non-trivial change.
3. Keep changes small and scoped. Do not introduce broad frameworks or dependencies without an ADR.
4. Keep shared logic platform-neutral; isolate native handles and `unsafe` code in the appropriate adapter. Explain every unsafe block and lifetime invariant.
5. Do not invent API guarantees. Link primary documentation and include native test evidence.
6. Run formatting, clippy and tests where Rust is available. Run native integration tests on the real target OS for platform behavior.
7. If native tests cannot run, say **not verified** and include the exact missing environment/result. A successful cross-compile is not runtime validation.
8. Never use private/undocumented APIs, privilege escalation, hidden/minimize workspace fallback, shell execution from window metadata/config, telemetry, or unapproved network calls.
9. Update capability matrix, tests and docs with any behavior change. Record workspace results using `docs/PLATFORM-SPIKES.md`.
10. Before handing off, summarize files changed, commands/results, remaining limitations and commit/branch identifier. Do not claim success beyond the evidence.

## Definition of done

- Acceptance criteria have a test or an explicit reason they cannot be automated.
- Failure/permission-denied/stale-window/shutdown paths are handled.
- Windows and macOS claims are backed by native tests on the supported OS/build.
- `cargo fmt --check`, clippy and relevant tests pass, or the environment limitation is recorded.
- Diff is focused; no sensitive logs, screenshots, credentials or unrelated binaries are committed.
