# REA-Assisted Research & Behavior Specification Plan

**Purpose:** Produce evidence-backed, original behavioral specifications for the subset of Hyprland-like UX that can be implemented through Windows/macOS app APIs.  
**Target REA:** [morluto/rea](https://github.com/morluto/rea) (“Reverse Engineer Anything”).  
**Important:** REA is an analysis workflow/toolbox (MCP/CLI + analyzers), not a magic clone generator. This workspace has not installed or run REA. The steps below are a proposed controlled workflow.

## 1. Legal and ethical boundaries

- Hyprland is publicly available source software; prefer source/documentation review over binary disassembly. Review current upstream license and notices before copying any code. This project should implement original Rust logic and preserve required notices if any upstream material is ever incorporated.
- Use REA only on binaries, systems and captures you own or have explicit permission to analyze. Do not bypass access controls, extract credentials, or inspect unrelated proprietary software.
- Do not copy Hyprland branding, artwork, code, private plugin internals, or falsely advertise compatibility. Record inspiration/reference behavior, not source-code-derived implementation text.
- Obtain legal review before distribution, especially for names, config compatibility claims, dependencies, licenses and trademarks.

## 2. Research questions (scope)

1. Which Hyprland user concepts are platform-independent? (layouts, focus direction, rules, gaps, workspaces, floating, keybindings)
2. What does each behavior mean precisely at the user level, independent of its compositor implementation?
3. Which concepts map to Windows 11/macOS APIs, are degraded, or are impossible without owning the compositor/session?
4. What expected behavior can be covered by a deterministic shared engine and what must remain platform-specific?
5. Which edge cases have observable evidence, and which are assumptions requiring a spike?

## 3. Evidence-first method (inspired by REA's evidence-oriented workflow)

### Phase A — define the target, version and authorization

- Pin a Hyprland release/commit and documentation version; record URLs, access date and license.
- Pin REA version/commit if used; record runtime/tool versions.
- Confirm analysis authorization and define artifact scope. No credentials or personal data in fixtures.
- Create a research ledger with question, hypothesis, method, raw observation, evidence locator, confidence, limitations and reviewer.

### Phase B — collect authoritative public behavior

- Review upstream docs/config reference, changelog, sample config and public tests. Prefer documentation and source-level interfaces over disassembly.
- For relevant behaviors, gather multiple evidence types: docs, code/test reference, observable run on an authorized Linux test VM, and user acceptance examples.
- Use REA against the authorized binary only where static/dynamic questions remain unanswered. Preserve exact tool/version, command, input hash and output excerpt; every conclusion must link to evidence (symbol/function/file/line or runtime observation).
- Separate **observed fact**, **interpretation**, **design decision** and **unknown**. Never promote a model-generated guess to fact.

### Phase C — translate into platform-neutral behavior specs

For each concept create a one-page spec with:

- User-visible intent and terminology.
- Preconditions/state transitions and expected result.
- Edge cases (zero/one/many windows, minimized, dialog, fullscreen, multi-monitor, DPI/scale change, close during apply, permission revocation).
- Acceptance examples and property/invariant tests.
- Platform mapping: Windows = supported/partial/unavailable; macOS = supported/partial/unavailable; evidence and limitation.
- Compatibility decision: implement, adapt, defer or explicitly reject.

### Phase D — prototype on real platform hardware

Use small throwaway Rust probes, not the main app, to establish API facts. Test on Intel Mac running Ventura 13+ and Windows 11 x64; capture OS build, app identity/signing, scale, hardware, permission state, expected vs actual geometry and native errors. For workspaces, separately test detection, switch, move/associate and observation via public APIs. If any required operation is unverified/unsupported, mark workspace unavailable on that OS. Never substitute hide/minimize or private APIs. Delete sensitive screenshots/captures after extracting minimal observations.

### Phase E — review and sign-off

- One reviewer validates evidence; another reviewer validates OS/API feasibility.
- Product approves intentional deviations from Hyprland.
- Legal/security reviews licensing, permissions and data handling.
- Add approved behavior tests to `QUALITY.md`; update ADRs when evidence changes a decision.

## 4. Research inventory template

| ID | Question/feature | Source/version | Method | Evidence locator/hash | Observation | Confidence | Limitation/next check | Decision |
|---|---|---|---|---|---|---|---|---|
| R-001 | Example: BSP insertion after window spawn | upstream docs + fixture | docs/test | URL/commit/test name | Pending | Low | Verify on pinned version | Pending |

Confidence scale: **High** = reproducible primary evidence + independent corroboration; **Medium** = primary evidence but limited coverage; **Low** = inference/one observation. Do not ship low-confidence platform behavior as guaranteed.

## 5. REA usage checklist (when available)

- Verify the repository identity is exactly `morluto/rea`; inspect the README, license, version and requested permissions before setup. Avoid lookalike repos/installers.
- Pin and review the source/package; use an isolated environment. Do not run downloaded scripts with broad permissions blindly.
- Confirm analyzer requirements/tool licenses (e.g. Ghidra/IDA/Hopper) and do not assume REA grants licenses.
- Choose the narrowest analysis mode. Exclude user data, credentials, unrelated binaries and sensitive network traces.
- Record analyzer failure/partial coverage; a missing result is not evidence that behavior does not exist.
- Use REA's evidence as an index into artifacts, then independently verify. Store citations/locators and hashes, not large binaries or raw sensitive logs.

## 6. Initial feature mapping hypothesis (to be validated)

| Hyprland-like concept | Shared-core target | Windows 11 feasibility | Intel macOS feasibility |
|---|---|---|---|
| BSP/master-stack/columns/monocle | Pure geometry planner | High for ordinary movable windows | Medium/high via AX; per-app variance |
| Focus directional navigation | Spatial neighbor selection | Medium; activation restrictions | Medium; AX/focus behavior varies |
| Window rules / float | Eligibility policy | Medium/high from app/window metadata | Medium; AX metadata availability varies |
| Gaps/margins | Layout geometry | High | High |
| Global shortcuts | Action bindings | Medium; registration conflicts/reserved keys | Medium; permissions/API behavior to validate |
| Workspaces | Optional native OS desktop/space actions; no app-owned groups | Unverified; expose only if required public-API actions pass build-specific spikes; otherwise disable | Unverified; expose only if required public-API actions pass on Intel Ventura 13+; otherwise disable |
| Decorations, blur, animation, compositor effects | Not MVP | Not equivalent from ordinary app | Not equivalent from ordinary app |
| Wayland compositor protocol/session | Not in product scope | Not applicable | Not applicable |

This table is a hypothesis, not an API guarantee. Update it only with primary platform evidence and prototype results. Workspace operations must pass the complete selected contract (detect active/target, switch, move focused window, observe/report); never use minimize/hide or private APIs as a fallback. Auto-tiling remains disabled until explicit onboarding opt-in.

## 7. Deliverables from research phase

- Versioned behavior catalog for MVP features.
- Feature-to-platform capability matrix with evidence links.
- Licensing/third-party inventory.
- Windows and macOS API spike reports.
- Open-issues list with risks and approved deviations.
- Executable fixtures/golden tests for shared layout behavior (no proprietary source copied).
