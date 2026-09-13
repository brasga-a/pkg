# ADR-015: CLI contract

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Use noun/verb-style CLI commands while keeping internal Rust APIs and plan serialization unstable through MVP.

## Alternatives considered

- **Stable public Rust library now:** not selected for the current architecture.
- **Daemon RPC first:** not selected for the current architecture.

## Why this decision

- Stable user mental model.
- Avoid premature API commitment.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit when third-party programmatic integration becomes a concrete requirement.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
