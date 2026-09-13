# `pkg remove`

Detach a package from the selected profile and remove pkg-owned state when safe.

## Syntax

```bash
pkg remove <package>
```

Example:

```bash
pkg remove ripgrep
pkg remove ripgrep --dry-run
```

## Removal model

Removal is ownership-based, not path-pattern-based.

```text
installed package
 -> resolve active store object
 -> determine reverse dependencies
 -> create RemovePlan
 -> update profile generation
 -> commit state change
 -> mark unreachable store object
 -> optional immediate cleanup / later GC
```

## Dependency protection

If another installed package depends on the target, the default behavior is to refuse removal and show the dependency chain.

Future force semantics must be explicit and must not silently leave a broken profile.

## Store deletion

`pkg remove` does not need to immediately delete the store tree.

A safe sequence is:

1. remove activation;
2. update logical installed state;
3. mark object unreachable;
4. let `pkg gc` reclaim it.

This improves rollback/recovery.

## Configuration

MVP packages live inside pkg-owned stores, so package-owned configuration inside the store disappears with the store object.

Future host/user configuration outside the store must have an explicit retention policy. Never delete arbitrary user data by package-name convention.

## Postconditions

Success means the package is no longer active/installed in the selected profile and no live package state depends on its activation.

## Errors

- package not installed;
- reverse dependency conflict;
- transaction lock unavailable;
- activation switch failure;
- state inconsistency requiring `pkg doctor`.
