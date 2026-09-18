# ADR-020: v1 scope for native signing and store identity

## Status

Accepted

## Context

The v1 lifecycle contract needs a stable support claim, but two design choices
remain broader than the verified install path: a pkg-native repository signing
format and the final choice between content-addressed and artifact-derived
store identities. Treating either as settled would overstate the public API and
make existing source-specific trust evidence ambiguous.

## Decision

The v1 support claim explicitly excludes a pkg-native repository signing
format and does not promise a final content-addressed store identity scheme.
Source-native repository trust remains the supported path. Store identities in
the current implementation remain implementation details derived from verified
artifact and realization inputs; callers must use the recorded identity and
must not construct or compare it as a stable cross-version API.

No native signing key format, migration promise, or identity compatibility
guarantee is added by this ADR. Both decisions are deferred to a post-v1 ADR
that must include migration and trust-rotation evidence before changing the
support claim.

## Consequences

- M5 can state the exclusions explicitly instead of leaving DEC-021/DEC-022
  open blockers for the published v1 surface.
- Repository adapters continue to preserve their own signature and freshness
  evidence; no universal signature verifier is implied.
- Garbage collection and recovery rely on database ownership and recorded
  paths, not on a promised global content-addressed namespace.

## Revisit conditions

Revisit when a pkg-native repository service or a cross-version store migration
is part of a concrete release plan. The replacement ADR must define trust
rotation, downgrade behavior, migration recovery and compatibility tests.
