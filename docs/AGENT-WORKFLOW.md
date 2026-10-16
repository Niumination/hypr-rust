# Engineering Agent Workflow (Karpathy-inspired)

This is a practical workflow drawing on widely discussed engineering heuristics associated with Andrej Karpathy—understand before changing, use explicit specifications, keep the system legible, build the smallest working slice, and verify behavior empirically. It is **not** a claim that a private/external “Karpathy skill” was installed or invoked.

## Working loop

1. **State the task and invariant.** Identify the user-visible behavior, expected result, constraints and out-of-scope work. Link to a PRD requirement or issue.
2. **Inspect before editing.** Read the relevant modules, tests, platform docs and recent failures. Trace the complete data path, not just the file that appears responsible.
3. **Write a concrete plan.** Include files/modules, risks, tests, rollback and OS-specific implications. For ambiguous product behavior, ask rather than infer silently.
4. **Make the smallest coherent change.** Prefer one vertical slice and explicit types over speculative frameworks. Avoid widening unsafe FFI surface.
5. **Run the right evidence.** Unit tests for pure logic; platform-native integration tests for OS behavior. Report exact commands, environment, results and what was not tested.
6. **Review failure modes.** Check permission denied, stale handles, display changes, minimized/fullscreen/dialog windows, event loops, cancellation and clean shutdown.
7. **Document facts vs assumptions.** Update tests, docs, capability matrix and ADR when decisions/evidence change.

## Repository rules for contributors and coding agents

- Do not call the app “Hyprland for Windows/macOS” in product copy; use the approved product description and state limitations.
- Never invent API guarantees. Link primary OS docs or attach a reproducible native test.
- Do not use private Apple APIs or request elevated/admin execution as a workaround without approval.
- Keep `unsafe` in narrow adapter modules; every unsafe block must explain pointer/resource lifetime and invariants.
- No shell execution from user-supplied config or window title. No hidden network access or telemetry.
- Preserve failures in `ApplyReport`; never treat an attempted OS call as a successful placement.
- If unable to run native tests, label changes unverified and do not claim the platform works.
- Before changing behavior, add a failing test or a concise reproducible case where possible.
- Avoid broad refactors in bug-fix PRs. Make review easy: focused diff, typed errors, meaningful tests.

## Definition of done for an implementation issue

- Acceptance criteria are met and linked to tests.
- `fmt`, clippy and relevant test suites pass.
- Native platform tests run, or an explicit blocker/verification gap is documented.
- Failure handling, permission behavior and shutdown are covered.
- User-facing docs/capability matrix are updated if behavior or requirements changed.
- No unrelated changes, secrets, sensitive captures or unreviewed dependency additions.

## Prompt template for an implementation agent

```text
Task: <one behavior>
Requirement: <PRD/issue link and acceptance criteria>
Constraints: <supported OS/version, permissions, non-goals>
Inspect: trace current flow and report relevant modules before editing.
Plan: propose smallest implementation and tests; do not edit until plan is coherent.
Implement: preserve platform boundaries and typed failures; avoid private APIs.
Verify: run exact commands and native tests; report environment and limitations.
Review: inspect diff for unsafe code, loops, stale handles, privacy and shutdown.
Deliver: summarize changed files, behavior, test evidence, known risks, follow-ups.
```
