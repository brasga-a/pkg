# Research Intake

## Problem statement

Linux distribution packages encode more than compressed files. They encode dependency vocabularies, lifecycle scripts, policy assumptions, filesystem locations, ABI expectations and repository trust models.

A cross-distro package manager therefore cannot equate:

```text
"can parse artifact" == "can safely install artifact on any distro"
```

## Questions driving architecture

1. Which semantics are intrinsic to the artifact and which belong to the source distribution?
2. What subset can be normalized across Debian, RPM and ALPM?
3. Which compatibility checks can be verified from the host?
4. Which behaviors must be default-deny?
5. How can pkg coexist with native package ownership?
6. How should remote repository metadata map to one internal catalog?
7. How much dependency solving should be implemented before real evidence requires a SAT/PubGrub-class solver?
8. How should binaries be activated without creating ambiguous ownership?
9. What transaction evidence is necessary for recovery?
10. Which package classes are fundamentally unsafe for cross-distro installation?

## Working thesis

The portable unit is not a distro package itself. The portable unit is:

```text
artifact
+ normalized metadata
+ compatibility evidence
+ explicit integration policy
```

pkg should progressively classify packages:

- **portable** — works with bundled/host capabilities and isolated store;
- **adaptable** — works after safe deterministic adjustments;
- **host-integrated** — needs explicit host actions;
- **container-required** — source distro semantics are required;
- **unsupported** — unsafe or insufficiently understood.
