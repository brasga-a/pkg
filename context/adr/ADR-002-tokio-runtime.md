# ADR-002: Tokio runtime

## Status

Accepted

## Context

pkg is a cross-distribution Linux package manager whose initial safety boundary is an isolated user-space store. The decision must preserve coexistence with native package managers and avoid claiming semantic compatibility merely from archive-format support.

## Decision

Use Tokio for asynchronous HTTP, concurrent downloads and future process/network orchestration; domain semantics remain runtime-independent.

## Alternatives considered

- **async-std/smol:** not selected for the current architecture.
- **blocking-only architecture:** not selected for the current architecture.

## Why this decision

- Mature async I/O.
- reqwest integration.
- bounded concurrency primitives.

## Consequences

- The implementation and tests must encode this boundary explicitly.
- Features that would bypass the boundary require a new ADR or amendment.
- User-facing support claims must remain narrower than parser capabilities.

## Risks

- The selected approach may expose complexity later as broader repository/package classes are added.
- A safe default can reject packages that a native distribution package manager would successfully install.

## Revisit conditions

Revisit if measured complexity or binary/runtime constraints outweigh async repository/download benefits.

## Evidence

See `meta/decisions.md`, relevant `design/` documents and research notes.

## Confidence

HIGH
