# ADR-004: No implicit native package database mutation

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Do not modify dpkg/RPM/libalpm installed-state databases except through a future explicit native-provider feature.

## Alternatives considered

- **Write native DB directly:** not selected for the current architecture.
- **Invoke native manager as hidden side effect:** not selected for the current architecture.

## Why this decision

- Avoid corrupting host package ownership.
- Clear authority boundary.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit only with an explicit native-provider ADR.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
