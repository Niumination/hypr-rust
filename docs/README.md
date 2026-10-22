# Hypr-Rust — Engineering Documentation

**Status:** Interview baseline / pre-implementation; native macOS M1/M2 single-window proof (host-bound); native batch multi-window DEFERRED under strict no-partial ([ADR-0004](./adr/0004-batch-multiwindow-deferred-no-partial.md))  
**Document date:** 2026-10-17 (updated 2026-10-22)  
**Working name:** Hypr-Rust (placeholder; do not imply affiliation with Hyprland)

## Executive summary

Build a Rust application that reproduces the *tiling workflow and user-facing concepts* of Hyprland on Windows 11 and Intel Macs. This is **not** a port of the Wayland compositor: Windows and macOS do not expose a third-party compositor/session API equivalent to Wayland that this product can own. The feasible product is a **cross-platform tiling window manager/controller** that rearranges ordinary desktop windows through each OS's supported APIs.

- **Windows 11 x64:** tray app/settings UI; enumerate/observe top-level windows and request placement via Win32.
- **Intel macOS Ventura 13+:** menu-bar app/settings UI; use public Accessibility APIs with transparent permission onboarding.
- **Shared Rust core:** BSP, master/stack, monocle and columns/rows; deterministic layout engine, window/display model, configuration, event processing and tests.
- **Safety/product decisions:** auto-tiling only after explicit onboarding opt-in; platform-appropriate editable shortcuts; user rules for ordinary windows with mandatory system/app-self exclusions; quit leaves windows at their last position.
- **Workspace:** native Windows Virtual Desktop/macOS Spaces only, optional per platform and gated by proof using public APIs. Never emulate with minimize/hide. If unsupported, disable the workspace feature on that platform and explain why.
- **Explicitly out of scope for MVP:** replacement display server/compositor; Wayland protocol hosting on Windows/macOS; universal interception of every app; pixel-identical Hyprland rendering/effects; Linux support unless separately approved.

The product boundary is documented in [ADR-0001](./adr/0001-product-boundary.md); shared core/platform adapters in [ADR-0002](./adr/0002-core-and-platform-adapters.md); the native-workspace no-emulation policy in [ADR-0003](./adr/0003-native-workspaces.md); and the strict no-partial deferral of native batch multi-window in [ADR-0004](./adr/0004-batch-multiwindow-deferred-no-partial.md). The PRD records interview decisions; the other documents define architecture, REA-assisted research, validation and staged delivery.

## Document map

1. [Product requirements (PRD)](./PRD.md)
2. [Architecture and technical design](./ARCHITECTURE.md)
3. [Reverse-engineering / behavior research plan](./RESEARCH-PLAN.md)
4. [Quality, testing and release plan](./QUALITY.md)
5. [Roadmap and milestones](./ROADMAP.md)
6. [Native platform spike protocol](./PLATFORM-SPIKES.md)
7. [Current user device profile](./USER-DEVICE-PROFILE.md)
8. [macOS Tahoe next implementation stage](./MACOS-TAHOE-NEXT-STAGE.md)
9. [Read-only system information collection](./SYSTEM-INFO-COLLECTION.md)
10. [OpenCode / Hermes handoff plan](./AGENT-HANDOFF.md)
11. [ADR-0001: native window manager, not compositor port](./adr/0001-product-boundary.md)
12. [ADR-0002: shared core + OS adapters](./adr/0002-core-and-platform-adapters.md)
13. [ADR-0003: native workspace capability gate; no hide/minimize emulation](./adr/0003-native-workspaces.md)
14. [Agent workflow inspired by Karpathy's engineering heuristics](./AGENT-WORKFLOW.md)
15. [ADR-0004: batch multi-window deferred under strict no-partial requirement](./adr/0004-batch-multiwindow-deferred-no-partial.md)

## Non-goals and important caveats

1. “Rebuild Hyprland” here means an original, compatible-in-spirit tiling experience—not copying Hyprland code, branding, assets, plugin internals or claiming compatibility where none exists.
2. Hyprland is a Wayland compositor. Its compositor duties (display server, input protocol, rendering/composition, surfaces, security boundaries and session integration) are not achievable by an ordinary Windows/macOS app using their public window-management APIs.
3. OS APIs impose behavioral differences. Some windows cannot be moved/resized, macOS requires Accessibility approval, and focus/activation rules differ. Treat each platform's capabilities honestly in product/UI/docs.
4. The request names “REA” as the viral GitHub reverse-engineering project. It is treated as **morluto/rea — Reverse Engineer Anything**. REA is a research toolchain/MCP, not an implementation framework and not a substitute for validating observations. This plan does not claim REA was installed or run in this workspace.

## Current implementation state

The upstream `main` branch contains the platform-neutral layout-core scaffold (fmt/clippy/6 tests passing) plus the merged M1 Rust macOS AX adapter and M2 keyboard-first plan (merged; `ax-keyboard` select/move/resize/raise on the registered self-owned fixture). Native M2 (Jalur A) PASS is host-bound (ThinkPad X13 Yoga Gen 1, macOS Tahoe 26.7.1, self-owned fixture) and does not validate Apple hardware or older macOS. A read-only 2/3-window probe confirmed AX enumeration and geometry reads with no AX writes. Native **batch multi-window application is DEFERRED / BLOCKED** under the strict no-partial requirement ([ADR-0004](./adr/0004-batch-multiwindow-deferred-no-partial.md)); pure layout planning may continue. PR #1 (Swift spike) and PR #2 (min-size constraints) remain open. The macOS spike reports ordinary AX control on a self-owned test window and disables Spaces. The host label conflict (system `MacBookPro16,2` vs owner ThinkPad X13 Yoga) is reconciled in the M1 report §1. This remains **not a usable desktop app**: executor, Windows adapter, tray/menu-bar UI, settings UI, global hotkeys and packaging are outstanding. See [user device profile](./USER-DEVICE-PROFILE.md), [macOS next stage](./MACOS-TAHOE-NEXT-STAGE.md), and [handoff plan](./AGENT-HANDOFF.md).

## Definition of success for the first usable release

The planned app will manage supported windows with shared layouts and native adapters. The current implementation milestone is a narrow macOS Tahoe 26.7.1 Rust AX proof on the user's ThinkPad X13 Yoga Gen 1; this host-specific result does not establish Apple hardware or older macOS support. Windows 11 native work is deferred until the device boots Windows. Native macOS workspaces remain disabled. Product-wide compatibility claims wait for platform evidence.