# Transaction Model

Every mutating operation creates a durable transaction record.

## Install states

```text
Planned
  -> Fetching
  -> Verified
  -> Staging
  -> Prepared
  -> Promoting
  -> Activating
  -> Committing
  -> Completed
```

Failure leads to `FailedRecoverable` or `FailedClean`.

## Atomicity boundary

Filesystem and SQLite cannot form one portable atomic transaction. Therefore use an evidence protocol:

1. create transaction row;
2. stage on same filesystem as store;
3. fsync critical metadata where required;
4. atomic rename staging -> final store object;
5. create new activation generation;
6. atomically switch profile;
7. commit installed-state references;
8. mark transaction completed.

Startup recovery reconciles transaction journal, store object existence, profile generation and DB.

## Remove

Removal first detaches activation/state references. Physical deletion may be immediate or GC-driven.

## Dry run

InstallPlan must contain all expected package/store/activation mutations before transaction execution.
