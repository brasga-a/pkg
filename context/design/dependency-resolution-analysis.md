# Dependency Resolution Analysis

Dependency resolution across Debian, RPM and ALPM is not a single SemVer problem.

Complications:
- ecosystem-specific version ordering;
- alternative dependencies;
- virtual Provides;
- architecture qualifiers;
- RPM boolean dependencies;
- package conflicts/replaces;
- file/SONAME capabilities;
- optional/recommended relationships;
- source-repository priorities;
- host-provided capabilities.

## Strategy options

### Greedy resolver
Useful only for first controlled catalog fixtures. Easy to implement, poor conflict explanation.

### PubGrub-style solver
Good human-readable failure explanations and version constraints. Requires adaptation because distro dependency languages exceed simple package/version requirements.

### General SAT
Most expressive but increases modeling/debug complexity.

## Recommendation

Use a two-stage design:

```text
source dependency expression
      ↓
normalized constraint IR
      ↓
solver domain
```

Do not wire source metadata directly to a solver library.

Milestone 1 needs no remote solver. Later MVP can use PubGrub-class solving for normalized package/version constraints while capability resolution is an outer provider-selection step. Escalate to richer SAT only when test corpus demonstrates a real limitation.
