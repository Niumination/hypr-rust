# OpenCode / Hermes Agent Handoff Plan

This plan maps the current workspace snapshot to the user's two-machine workflow. It deliberately separates Windows and macOS investigations so two agents can work in parallel without editing the same platform files.

## Snapshot truth

At handoff, the repository contains:

- Product/architecture/quality docs and ADRs.
- `crates/layout-core/`: platform-neutral layout planner source and unit tests.
- `docs/PLATFORM-SPIKES.md`: evidence protocol for native testing.
- `AGENTS.md`: coding-agent instructions.

It does **not** contain a runnable desktop application, native OS adapter, settings/tray UI, global hotkeys, workspace integration, or verified build. The authoring environment lacked Rust tooling; the local `cargo fmt`, `cargo clippy`, and `cargo test` commands have not been run. Do not describe the scaffold as tested.

## Handoff sequence

### 0. Establish upstream

- Create an upstream repository and record its URL, default branch, license choice and initial commit SHA.
- Do not add a license file or assert a license until the owner selects it; the Rust scaffold intentionally leaves licensing undecided.
- Keep the downloaded workspace intact, including docs and `AGENTS.md`.
- Use separate branches and PRs for each platform spike. Avoid sharing the same working tree between two agents at once.

### 1. Baseline build (any Rust machine)

Run the commands in the root README. Fix compile/format/test defects in a focused PR before adding platform code. Attach exact Rust toolchain version and command output. If baseline fails, preserve the failure and diagnose it rather than layering native code on top.

### 2. Windows 11 x64 spike — dedicated Windows machine/agent

Suggested branch: `spike/windows11-window-and-desktop-apis`.

- First validate ordinary top-level window enumeration/move/resize and safe failure on unsupported/elevated windows.
- Investigate native Virtual Desktop requirements through public/reviewed APIs only: active/target detection, switching, moving the focused window, observing/reporting.
- Do not add private/undocumented interfaces, COM tricks without review, privilege escalation, or hide/minimize emulation to make a demo pass.
- Put code/evidence under a Windows-specific spike directory and result under `docs/platform-results/windows-<build>.md`.
- If any workspace operation is unsupported or not reproducible, mark workspace disabled for that build and continue the ordinary window-management spike.

### 3. Intel macOS Ventura 13+ spike — dedicated Intel Mac/agent

Suggested branch: `spike/macos13-ax-and-spaces-apis`.

- First validate Accessibility authorization, window enumeration/move/resize, app signing identity and permission revocation.
- Investigate Spaces requirements through public APIs only: active/target detection, switching, moving the focused window, observing/reporting.
- Do not use private WindowServer APIs, require disabling SIP, or use minimize/hide emulation.
- Put code/evidence under a macOS-specific spike directory and result under `docs/platform-results/macos-<version>.md`.
- If any required workspace operation is unsupported or not reproducible, mark workspace disabled for that version and continue the ordinary window-management spike.

### 4. Review and integration

For each PR, require:

- Exact base and head commit SHAs.
- Exact machine model, OS build, app identity/signing, permissions, test apps, Rust toolchain and commands.
- Results for each required operation, including failures and limitations.
- Tests/logs redacted of window titles and personal data.
- No claim of cross-platform parity based on one OS or one app.

After the platform spikes, choose architecture/dependencies from observed results, then build a minimal end-to-end vertical slice. Do not implement workspace UI until ADR-0003's complete contract passes on that OS.

## Prompt for either coding agent

```text
Read AGENTS.md, README.md, docs/PRD.md, docs/ARCHITECTURE.md, docs/PLATFORM-SPIKES.md,
and ADR-0003 before editing.

Task: <Windows 11 x64 spike OR Intel macOS Ventura 13+ spike>
Do not edit files belonging to the other platform spike.
First report repository state, toolchain, test availability and a short plan.
Use only reviewed public APIs. Do not use private APIs, hide/minimize workspace fallback,
admin/root escalation, or arbitrary shell commands from window metadata.
Run baseline Rust checks before platform changes. Test on the actual native OS.
Report each workspace operation independently: detect, switch, move focused window,
observe/report. If any is not proven, mark workspace unsupported for this OS/build.
Commit evidence and limitations, not just code. Do not claim unrun tests passed.
```

## Review loop with the project owner

After upstream changes, share the public repository URL plus branch/commit or PR URL. Review should target the exact commit and inspect code, test evidence and capability claims. If the repository is private or cannot be fetched, provide a patch/export and relevant logs with sensitive window data redacted. Never share tokens, credentials or private user captures through the repository.
