# ADR-009: Normalized solver IR

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Place a normalized dependency/capability constraint IR between source metadata and any solver library.

## Alternatives considered

- **Direct PubGrub types in adapters:** not selected for the current architecture.
- **Direct SAT clauses in format parsers:** not selected for the current architecture.

## Why this decision

- Avoid solver lock-in.
- Preserve source semantics.
- Testable normalization.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit only if the IR cannot represent required source semantics.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
