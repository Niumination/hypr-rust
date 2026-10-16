# ADR-0001: Build a Native Tiling Controller, Not a Cross-OS Compositor Port

- **Status:** Accepted as product interview baseline; native feasibility remains to be validated
- **Date:** 2026-10-16
- **Decision owners:** Product + platform leads

## Context

Hyprland is a Wayland compositor. A compositor is a display-server role, not merely a tiling algorithm. Hyprland's documentation describes Wayland compositors as autonomous display servers. Windows 11 and macOS use their own system window managers/compositors; ordinary third-party applications cannot replace them with a Hyprland-equivalent Wayland compositor through public APIs.

A Rust application can still implement the valuable user-facing subset: layout calculation, hotkeys, window observation, rules and requested placement of native windows. Windows exposes native window-positioning APIs such as `SetWindowPos`; macOS exposes Accessibility objects and actions, subject to user-granted trust and app support.

## Decision

Build a **cross-platform tiling window manager/controller** with a shared Rust domain/layout engine and separate native adapters. Describe it as “Hyprland-inspired” only with appropriate trademark/legal review and a clear non-affiliation statement. Do not claim it is a port, compositor, drop-in replacement, or exact-compatible implementation.

## Consequences

### Positive

- Technically feasible within public user-space APIs.
- Shared layout behavior is testable without OS APIs.
- Native safety, permissions and distribution can be handled per platform.

### Negative / accepted limitations

- Window moves can be denied or ignored; not every application is controllable.
- Focus rules and monitor/workspace handling differ by OS.
- Cannot provide compositor-level blur, borders, rendering, protocol compatibility or complete control of other windows.
- macOS needs Accessibility permission for relevant operations; permission/setup friction is part of the product.
- Native workspaces are governed by ADR-0003: optional per-platform capability only; never emulate with hide/minimize. The core release does not depend on workspace support.

## Alternatives considered

1. **Port Hyprland literally to Windows/macOS:** rejected; Wayland/compositor role and OS integration are incompatible with a normal application.
2. **Run Linux/Wayland under a VM/WSL:** rejected as the primary product; does not manage the host's native app windows consistently and is not a native host window manager.
3. **Fork/copy Hyprland internals:** rejected for this product; does not solve host OS API boundary and raises unnecessary maintenance/licensing/brand risks.

## Revisit when

- A supported OS exposes a stable public compositor/session replacement API, or product changes target to a Linux/Wayland distribution.
- A user explicitly requires Linux-hosted Hyprland in a VM rather than native host window management.
