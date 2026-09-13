# ADR-007: Repository snapshots

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Normalize source repository metadata into immutable local snapshots and atomically switch the active snapshot after successful verification.

## Alternatives considered

- **Mutable in-place catalog:** not selected for the current architecture.

## Why this decision

- Failed refresh keeps known-good state.
- Reproducible candidate view.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit if upstream protocol requires streaming semantics that cannot be represented safely.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
