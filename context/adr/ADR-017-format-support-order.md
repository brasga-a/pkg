# ADR-017: Format support order

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Implement `.deb` first, RPM second, ALPM third; generic tarball support is separate from distro-package semantics.

## Alternatives considered

- **All formats simultaneously:** not selected for the current architecture.

## Why this decision

- Controlled learning sequence.
- Deb format is simple to ingest.
- RPM adds rich capabilities.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit based on target application corpus.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
