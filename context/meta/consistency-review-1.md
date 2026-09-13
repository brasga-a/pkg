# Consistency Review 1

Review target: initial pkg architecture proposal.

## Passed

- Invariants agree with ADR decisions on isolated store and no native DB mutation.
- Maintainer script policy is consistently default-deny.
- Roadmap does not require remote repositories before local artifact vertical slice.
- Store and profile are distinct across architecture/design.
- Rootless default is consistent with host integration boundary.

## Follow-up

- Exact package version normalization remains intentionally open.
- Pkg-native signing protocol remains intentionally open.
- Container fallback is not part of v1 authority until an ADR accepts it.
