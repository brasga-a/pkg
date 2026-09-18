# `pkg gc`

Garbage-collect pkg-managed content that is no longer reachable.

## Syntax

```bash
pkg gc
pkg gc --dry-run
```

## Reachability roots

Store objects remain live when referenced by:
- active profile and its activation rows;
- installed package state;
- incomplete/recoverable transaction.

Retained generations, explicit pins and runtime leases are planned roots and
remain preserved until their state model is implemented. Unknown or divergent
state is retained instead of being inferred as unreachable.

Only recorded objects without those references may become a GC candidate.

## Targets

### Store GC
Remove unreachable pkg-owned store objects.

### Cache GC
Artifact and metadata cache collection is not part of the first implementation.
Cache entries remain available for verification and future explicit cache policy.

### Transaction cleanup
Remove abandoned temporary state only when a transaction record proves ownership and recovery is no longer required.

## Dry run

The implemented store-only dry run includes:

```text
STORE OBJECTS: 12 candidates
```

## Safety

GC never scans arbitrary user directories for files to delete.

Every deletion target must be inside the configured store root, match the path
recorded in the state database, and have no committed or incomplete transaction
reference. Directories not recorded by `pkg` are never deletion targets.

## Failure behavior

Partial GC failure does not invalidate installed state. Failed paths remain candidates for a later run.
