# ADR-014: Rootless default

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Install ordinary supported applications to user-owned XDG-compatible state/store/profile locations by default.

## Alternatives considered

- **Root daemon:** not selected for the current architecture.
- **System-wide default:** not selected for the current architecture.

## Why this decision

- Reduced blast radius.
- No sudo UX.
- Coexists with OS.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit for a separately designed system mode.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
