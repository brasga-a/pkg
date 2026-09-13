# ADR-011: Maintainer scripts default-deny

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Parse lifecycle scripts as metadata but never execute foreign scripts automatically.

## Alternatives considered

- **Run scripts in shell:** not selected for the current architecture.
- **Run scripts with sudo:** not selected for the current architecture.

## Why this decision

- Largest host-mutation risk.
- Distro-specific assumptions.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit per declarative integration capability, not through a blanket enable flag.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
