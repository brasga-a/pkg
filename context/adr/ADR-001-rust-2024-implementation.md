# ADR-001: Rust 2024 implementation

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Implement pkg in Rust, edition 2024.

## Alternatives considered

- **Go:** not selected for the current architecture.
- **C++:** not selected for the current architecture.
- **TypeScript:** not selected for the current architecture.

## Why this decision

- Memory-safe systems implementation.
- Strong parsing/filesystem ecosystem.
- Direct Linux API access.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit only for a measured blocker that prevents required Linux/package functionality.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
