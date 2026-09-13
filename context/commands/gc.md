# `pkg gc`

Garbage-collect pkg-managed content that is no longer reachable.

## Syntax

```bash
pkg gc
pkg gc --dry-run
```

## Reachability roots

Store objects remain live when referenced by:
- active profile;
- retained profile generation;
- installed package state;
- incomplete/recoverable transaction;
- explicit pin.

Everything else may become a GC candidate.

## Targets

### Store GC
Remove unreachable pkg-owned store objects.

### Cache GC
Remove old artifact/metadata cache entries according to cache policy.

### Transaction cleanup
Remove abandoned temporary state only when a transaction record proves ownership and recovery is no longer required.

## Dry run

Output should include:

```text
STORE OBJECTS: 12 candidates, 1.8 GiB
ARTIFACT CACHE: 43 candidates, 930 MiB
TEMP: 3 candidates, 12 MiB
```

## Safety

GC never scans arbitrary user directories for files to delete.

Every deletion target must be inside a configured pkg-owned root and have provable ownership.

## Failure behavior

Partial GC failure does not invalidate installed state. Failed paths remain candidates for a later run.
