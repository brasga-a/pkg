# ADR-003: Pkg-owned isolated store

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Install supported package payloads into a pkg-owned versioned store and activate them separately.

## Alternatives considered

- **Direct `/usr` install:** not selected for the current architecture.
- **Convert to native package:** not selected for the current architecture.
- **Container-only:** not selected for the current architecture.

## Why this decision

- Avoid native ownership conflicts.
- Side-by-side versions.
- Deterministic removal.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit if real package corpus proves isolated relocation unusable for the target application class.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
