# Hypr-Rust — Engineering Documentation

**Status:** Interview baseline / pre-implementation; feasibility validation pending  
**Document date:** 2026-10-17  
**Working name:** Hypr-Rust (placeholder; do not imply affiliation with Hyprland)

## Executive summary

Build a Rust application that reproduces the *tiling workflow and user-facing concepts* of Hyprland on Windows 11 and Intel Macs. This is **not** a port of the Wayland compositor: Windows and macOS do not expose a third-party compositor/session API equivalent to Wayland that this product can own. The feasible product is a **cross-platform tiling window manager/controller** that rearranges ordinary desktop windows through each OS's supported APIs.

- **Windows 11 x64:** tray app/settings UI; enumerate/observe top-level windows and request placement via Win32.
- **Intel macOS Ventura 13+:** menu-bar app/settings UI; use public Accessibility APIs with transparent permission onboarding.
- **Shared Rust core:** BSP, master/stack, monocle and columns/rows; deterministic layout engine, window/display model, configuration, event processing and tests.
- **Safety/product decisions:** auto-tiling only after explicit onboarding opt-in; platform-appropriate editable shortcuts; user rules for ordinary windows with mandatory system/app-self exclusions; quit leaves windows at their last position.
- **Workspace:** native Windows Virtual Desktop/macOS Spaces only, optional per platform and gated by proof using public APIs. Never emulate with minimize/hide. If unsupported, disable the workspace feature on that platform and explain why.
- **Explicitly out of scope for MVP:** replacement display server/compositor; Wayland protocol hosting on Windows/macOS; universal interception of every app; pixel-identical Hyprland rendering/effects; Linux support unless separately approved.

The product boundary is documented in [ADR-0001](./adr/0001-product-boundary.md); shared core/platform adapters in [ADR-0002](./adr/0002-core-and-platform-adapters.md); and the native-workspace no-emulation policy in [ADR-0003](./adr/0003-native-workspaces.md). The PRD records interview decisions; the other documents define architecture, REA-assisted research, validation and staged delivery.

## Document map

1. [Product requirements (PRD)](./PRD.md)
2. [Architecture and technical design](./ARCHITECTURE.md)
3. [Reverse-engineering / behavior research plan](./RESEARCH-PLAN.md)
4. [Quality, testing and release plan](./QUALITY.md)
5. [Roadmap and milestones](./ROADMAP.md)
6. [Native platform spike protocol](./PLATFORM-SPIKES.md)
7. [OpenCode / Hermes handoff plan](./AGENT-HANDOFF.md)
8. [ADR-0001: native window manager, not compositor port](./adr/0001-product-boundary.md)
9. [ADR-0002: shared core + OS adapters](./adr/0002-core-and-platform-adapters.md)
10. [ADR-0003: native workspace capability gate; no hide/minimize emulation](./adr/0003-native-workspaces.md)
11. [Agent workflow inspired by Karpathy's engineering heuristics](./AGENT-WORKFLOW.md)

## Non-goals and important caveats

1. “Rebuild Hyprland” here means an original, compatible-in-spirit tiling experience—not copying Hyprland code, branding, assets, plugin internals or claiming compatibility where none exists.
2. Hyprland is a Wayland compositor. Its compositor duties (display server, input protocol, rendering/composition, surfaces, security boundaries and session integration) are not achievable by an ordinary Windows/macOS app using their public window-management APIs.
3. OS APIs impose behavioral differences. Some windows cannot be moved/resized, macOS requires Accessibility approval, and focus/activation rules differ. Treat each platform's capabilities honestly in product/UI/docs.
4. The request names “REA” as the viral GitHub reverse-engineering project. It is treated as **morluto/rea — Reverse Engineer Anything**. REA is a research toolchain/MCP, not an implementation framework and not a substitute for validating observations. This plan does not claim REA was installed or run in this workspace.

## Current implementation state

The workspace now contains a platform-neutral layout-core scaffold and a native spike protocol. It is **not yet a usable desktop app**: Windows/macOS adapters, tray/menu-bar UI, settings UI, global hotkeys, packaging and native workspace implementation are outstanding. The scaffold has not been compiled here because `rustc`/`cargo` were unavailable in the authoring environment; see the root `README.md` and have CI/your machine run the stated commands.

## Definition of success for the first usable release

A user can install the app on Windows 11 x64 or Intel macOS Ventura 13+, configure it in the settings UI, explicitly opt into auto-tiling, and manage supported windows with layouts and platform-appropriate shortcuts. The app leaves window geometry as last accepted on quit. Native workspace shortcuts are available only on platforms/builds where public-API behavior is verified; unsupported workspace features are clearly disabled, never emulated by hiding/minimizing windows.