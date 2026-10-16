# References & Assumptions

Sources checked on **2026-10-16**. The links below ground platform/product-boundary decisions; they do not establish every behavior in the proposed application.

## Primary references

- Hyprland Wiki, Wayland info: describes a Wayland compositor as an autonomous display server and documents the compositor boundary. https://wiki.hypr.land/
- Hyprland installation documentation: illustrates the compositor's Linux/Wayland dependency surface. https://wiki.hypr.land/Getting-Started/Installation/
- Hyprland upstream repository (review license at the exact commit used): https://github.com/hyprwm/Hyprland
- REA upstream repository (`morluto/rea`): https://github.com/morluto/rea
- Microsoft Learn, `SetWindowPos`: native API for positioning/resizing a Windows window; API success/semantics do not guarantee control over every target application. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
- Apple Developer Documentation, `AXUIElement`: public Accessibility object model for inspecting UI elements, attributes, actions and notifications; access is permission-gated and actual per-app behavior requires testing. https://developer.apple.com/documentation/applicationservices/axuielement

## Assumptions requiring validation

1. The user's “REA viral repo” means `morluto/rea`, not another repository named REA. Confirm identity before execution; do not install a lookalike.
2. Target scope is Windows 11 x64 and Intel macOS Ventura 13+, not Apple Silicon, Linux, or a VM-only setup.
3. Feature parity is defined at the user-workflow level, not compositor/protocol/rendering parity.
4. Native workspace integration is optional per platform and must pass the public-API feasibility gate in ADR-0003; no hide/minimize emulation is allowed.
5. Hyprland's current source/docs and license can change; pin a specific commit/tag and check notices before implementing from upstream observations.
6. UI framework, exact Windows 11 build matrix, representative test apps, shortcut bindings, and signing implementation remain undecided until native feasibility spikes. The workspace operation contract is fixed in ADR-0003: detect, switch, move the focused window, and observe/report.

## Limitations of this design note

This is a requirements/architecture proposal, not a completed source audit, REA run, API proof, working application or signed compatibility guarantee. All quantitative NFR targets are initial engineering targets and need measurement on reference hardware.