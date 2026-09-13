# ADR-008: Source-specific trust evidence

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Preserve Debian/RPM/ALPM trust/signature evidence instead of inventing one universal translated signature.

## Alternatives considered

- **Trust HTTPS only:** not selected for the current architecture.
- **Re-sign all metadata locally:** not selected for the current architecture.

## Why this decision

- Avoid semantic loss.
- Allows ecosystem-correct verification.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit when defining a pkg-native repository.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
