# Architecture

## Layers

```text
┌─────────────────────────────┐
│ CLI / user interaction      │
├─────────────────────────────┤
│ Application services        │
│ install/search/remove/sync  │
├─────────────────────────────┤
│ Planning                    │
│ resolver + compatibility    │
├─────────────────────────────┤
│ Domain                      │
│ package/capability/plan     │
├─────────────────────────────┤
│ Infrastructure              │
│ repos/http/formats/store/db │
├─────────────────────────────┤
│ Host integration boundary   │
│ links/desktop/services      │
└─────────────────────────────┘
```

## Primary pipeline

```text
Package Source
     |
   Fetch
     |
 Verify transport/cache identity
     |
 Format Adapter
     |
 Normalized Package
     |
 Resolver <---- Catalog
     |
 Compatibility Planner <---- Host Facts
     |
 Install Plan
     |
 Transaction Executor
   /    |      \
Store  State   Activation
```

## Architectural boundaries

### FormatAdapter

Understands archive syntax and ecosystem metadata. It does not decide host policy.

### Catalog

Exposes package candidates, versions, dependencies, provides/capabilities, source URLs and digests independent of source repository shape.

### Resolver

Chooses candidate versions based on normalized constraints. It must not mutate disk.

### CompatibilityPlanner

Checks whether normalized requirements can be met by:
- package closure;
- host-provided capabilities;
- optional native provider;
- explicit integration.

### TransactionExecutor

Owns staging, verification, promotion, state update and compensation/recovery records.

### Store

Owns immutable-ish versioned package trees.

### Activation/Profile

Owns user-visible command links and future desktop integration. It is distinct from the store so changing active versions does not rewrite payloads.

## Suggested crate/workspace split after MVP

```text
crates/
├── pkg-cli
├── pkg-core
├── pkg-formats
├── pkg-repo
├── pkg-resolver
├── pkg-store
├── pkg-state
├── pkg-host
└── pkg-testkit
```

Do not split into workspace crates until module boundaries are proven by implementation pressure.
