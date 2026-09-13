# ADR-006: SQLite local state

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Use SQLite for pkg metadata, transactions, repositories and activation state.

## Alternatives considered

- **JSON files:** not selected for the current architecture.
- **redb:** not selected for the current architecture.
- **RocksDB:** not selected for the current architecture.

## Why this decision

- Transactional local DB.
- Portable tooling.
- Simple deployment.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit for measured scale/concurrency limitations.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
