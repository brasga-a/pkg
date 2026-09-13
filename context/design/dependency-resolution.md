# Dependency Resolution

## Constraint IR

Dependencies should support:
- all-of;
- any-of;
- capability requirement;
- version predicate;
- architecture predicate;
- conflicts.

```text
All(
  Capability("openssl", >= X),
  Any(Capability("gtk3"), Capability("gtk4"))
)
```

The original source expression remains attached for diagnostics.

## Resolution phases

1. filter candidate architecture;
2. select candidate package versions;
3. expand package dependencies;
4. satisfy virtual/capability requirements;
5. apply conflicts;
6. test host-provided capabilities;
7. return closure or explanation.

## Host capabilities

Host satisfaction is evidence-based:
- executable available;
- ELF SONAME available in trusted linker paths;
- libc/kernel minimum;
- optional native package query provider.

A package name existing in the host native database is useful evidence but not universal truth.

## Locking

Future remote installs should persist the chosen candidate IDs/digests as an install lock record so repeated execution does not silently resolve to a different artifact.
