# `pkg upgrade`

Resolve newer acceptable versions for installed packages and apply a transactional upgrade plan.

## Syntax

```bash
pkg upgrade
pkg upgrade <package>
pkg upgrade --dry-run
pkg upgrade --jobs 4
# alias:
pkg update
```

## Concurrent acquisition

`--jobs <N>` accepts values from 1 to 16 (default: 4). Upgrade roots and each
already discovered dependency frontier are acquired by Tokio tasks bounded by
that value and deduplicated by SHA-256 digest. Network completion order never
defines resolution, installation or state writes; `--jobs 1` keeps a
reproducible serial acquisition mode.

## Precondition

Repository snapshots should already exist. The command may warn that metadata is stale but should not silently conflate upgrade with repository refresh unless CLI policy explicitly chooses that behavior.

A clean mental model is:

```bash
pkg repo sync
pkg update
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

Because old store objects are not mutated in place, the implementation retains
enough state for profile rollback. Use `pkg rollback` to select the previous
generation after a successful upgrade.

## Dependency changes

An upgrade may:
- add dependencies;
- replace providers;
- create command conflicts;
- become incompatible with current host.

Any of these must be visible in the plan.

## Failure

If activation of the new generation fails, the previous active generation remains authoritative.
