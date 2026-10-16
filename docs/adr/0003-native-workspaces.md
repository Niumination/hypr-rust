# ADR-0003: Native Workspaces Only; No Hide/Minimize Emulation

- **Status:** Accepted product contract; implementation capability remains unverified per platform
- **Date:** 2026-10-17
- **Decision owners:** Product + Windows/macOS platform leads

## Context

The user wants keyboard shortcuts for workspaces, but cross-platform workspace semantics are ambiguous. An app-owned logical grouping is not the same as Windows Virtual Desktops or macOS Spaces. Ordinary window-management APIs do not guarantee uniform desktop switching or moving app windows between OS desktops. Hiding/minimizing windows to simulate workspace changes can disrupt apps, notifications, focus and user expectations.

## Decision

1. “Workspace” means the OS-native Windows Virtual Desktop or macOS Space. The app will not create logical workspaces of its own.
2. Workspace controls are optional and capability-gated independently for each OS/build. Expose them only after the required operations are verified using reviewed public APIs on the declared support matrix.
3. If the required contract cannot be verified, disable workspace actions on that platform and explain why. This does not block the core tiling manager release.
4. If an individual operation is denied or unsupported, leave the affected window unchanged and report the outcome. Never fall back to minimize, hide, private APIs, or privilege escalation.
5. At startup, follow the OS-selected active desktop/space; do not force a switch. Do not persist/restore native handles or force workspace restoration.
6. On quit, stop observation/management and leave windows at their last OS-accepted geometry; do not restore initial geometry.

## Required feasibility evidence

For each OS/build, the following complete set is required before workspace shortcuts can be exposed:

- Detect/identify the active native desktop/space and the target desktop/space.
- Switch to a selected desktop/space.
- Move the currently focused window to a selected desktop/space.
- Observe user-initiated changes and report/recover safely from denied or stale operations.
- Document app-signing/permission requirements and side effects.

Each operation must have reproducible native test evidence on the declared OS build. A partial API, undocumented/private API, minimize/hide fallback, or manual-only workaround is not sufficient to claim support. If any required operation fails, workspace shortcuts are disabled on that platform; core tiling remains available.

## Consequences

### Positive

- No contradictory concept of “logical workspace” versus OS desktop.
- No surprising minimize/hide behavior.
- Core tiling can ship even if workspace integration is unavailable on one or both OSes.
- Capability and failure behavior are explainable and testable.

### Negative / accepted limitations

- Workspace shortcuts may be absent on one platform or all platforms.
- Cross-platform workspace parity is not promised.
- User remains responsible for native desktop organization where the app's capability report says unsupported.

## Alternatives rejected

- **App-managed logical groups that hide inactive windows:** rejected for initial scope because hide/show semantics and restore guarantees differ by OS and application.
- **Minimize as a fallback:** rejected because it changes app state and can disrupt user workflows.
- **Private/undocumented APIs or elevated helper:** rejected for MVP due to compatibility, security and maintenance risk.
