# `pkg doctor`

Inspect and explain inconsistencies in pkg-managed state.

## Syntax

```bash
pkg doctor
```

Optional future mode:

```bash
pkg doctor --repair
```

Automatic repair must be conservative and separately documented.

## Checks

### State database
- schema version;
- incomplete transactions;
- dangling package/store references;
- duplicate active activation.

### Store
- missing referenced objects;
- unexpected mutation/digest mismatch where tracked;
- unreachable objects;
- invalid ownership/permissions.

### Profiles
- dangling symlinks;
- activation conflicts;
- missing active package;
- wrapper target missing.

### Repository state
- missing snapshot;
- stale metadata;
- invalid cached index;
- trust state.

### Host compatibility
- previously satisfied SONAME no longer available;
- dynamic linker/libc drift;
- architecture mismatch after migrated state.

## Output classes

```text
OK
WARN
ERROR
RECOVERABLE
MANUAL_ACTION_REQUIRED
```

## Repair philosophy

Diagnosis is always available. Repair is opt-in and must never delete data whose ownership is uncertain.

Examples of safe repair:
- remove temp staging directory belonging to a completed/failed known transaction;
- rebuild profile link from authoritative state.

Unsafe automatic repair:
- delete unknown filesystem content;
- rewrite native package-manager state.
