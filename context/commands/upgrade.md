# `pkg upgrade`

Resolve newer acceptable versions for installed packages and apply a transactional upgrade plan.

## Syntax

```bash
pkg upgrade
pkg upgrade <package>
pkg upgrade --dry-run
```

## Precondition

Repository snapshots should already exist. The command may warn that metadata is stale but should not silently conflate upgrade with repository refresh unless CLI policy explicitly chooses that behavior.

A clean mental model is:

```bash
pkg update
pkg upgrade
```

## Flow

```text
installed set
 + repository snapshots
 -> candidate resolution
 -> compatibility checks
 -> UpgradePlan
 -> fetch/verify new artifacts
 -> install new store objects
 -> build new profile generation
 -> atomic profile switch
 -> commit state
 -> old objects remain GC-eligible
```

## Rollback property

Because old store objects are not mutated in place, upgrade architecture should preserve enough state for future profile rollback.

Rollback UX is deferred until profile generations are fully accepted.

## Dependency changes

An upgrade may:
- add dependencies;
- replace providers;
- create command conflicts;
- become incompatible with current host.

Any of these must be visible in the plan.

## Failure

If activation of the new generation fails, the previous active generation remains authoritative.
