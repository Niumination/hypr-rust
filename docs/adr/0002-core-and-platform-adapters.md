# ADR-0002: Shared Rust Core with Native Platform Adapters

- **Status:** Proposed
- **Date:** 2026-10-16
- **Decision owners:** Rust lead + Windows/macOS leads

## Context

Layout semantics and native window control have different portability boundaries. Tiling algorithms, rules and state transitions can be pure Rust. Window discovery, permission, event subscriptions, geometry conversion, focus and move/resize are OS-specific and subject to platform failures.

## Decision

Use a Cargo workspace with a platform-neutral domain/layout/engine core and independently testable Windows and macOS adapter crates. Platform handles remain private to adapters. Core computes immutable plans; adapters apply them and return per-operation reports. Dependency/framework choice remains open until native proof-of-concept spikes.

## Consequences

- Core algorithms can be property tested and fuzzed on any CI host.
- OS-specific failures are explicit and do not leak native types into shared code.
- Some abstraction overhead is accepted to enable reliable platform test doubles.
- No promise of identical behavior where native APIs differ.

## Alternatives

- One monolithic crate with scattered `cfg` branches: rejected due to blurred ownership and difficult platform testing.
- Separate apps/repositories: deferred; shared semantics and config would drift before product stability.
- Adopt a large cross-platform GUI/windowing abstraction as the entire solution: rejected; it cannot remove native permission/control differences.
