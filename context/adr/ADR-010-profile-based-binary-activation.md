# ADR-010: Profile-based binary activation

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Expose commands through profile links/wrappers pointing to immutable store objects.

## Alternatives considered

- **Copy binaries into ~/.local/bin:** not selected for the current architecture.
- **Mutate store in place:** not selected for the current architecture.

## Why this decision

- Atomic version switch.
- Conflict visibility.
- Rollback.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit for platforms/filesystems where links are unavailable.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
