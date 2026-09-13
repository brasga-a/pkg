# ADR-005: Artifact adapter boundary

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Use format adapters that parse source artifacts into a normalized internal model without host mutation.

## Alternatives considered

- **One parser with format conditionals everywhere:** not selected for the current architecture.

## Why this decision

- Separates syntax from policy.
- Enables deb/rpm/alpm support.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit if two real formats demonstrate a better shared primitive.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
