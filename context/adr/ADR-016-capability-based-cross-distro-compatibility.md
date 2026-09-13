# ADR-016: Capability-based cross-distro compatibility

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Translate source dependency vocabulary into capability candidates and verify them with package closure/host evidence; package-name aliases alone never prove satisfaction.

## Alternatives considered

- **Static Debian↔Fedora↔Arch name table as authority:** not selected for the current architecture.

## Why this decision

- Names differ across distros.
- ABI matters.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit only if a narrower verified abstraction replaces capabilities.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
