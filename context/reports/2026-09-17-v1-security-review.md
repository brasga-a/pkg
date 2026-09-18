# Security review — rootless install and runtime surface

## Scope

This review covers the implemented local install, repository cache, store,
profile, runtime, generation, rollback, garbage collection, doctor, upgrade
and MCP paths. It is bounded by the published support matrix and does not
claim process sandboxing or system-wide installation.

## Findings

| Area | Evidence | Result |
|---|---|---|
| Archive traversal and expansion | parser, link-topology and extraction-limit tests | No unresolved blocker in the supported archive path. |
| Store and profile ownership | store path validation, unmanaged-content refusal and removal tests | Unknown regular files and foreign paths fail closed. |
| Native package databases | transaction gate test and explicit-host-integration policy | No implicit writes to `dpkg`, RPM or libalpm state. |
| User-space host integration | typed-link ownership, invalid-resource, conflict and recovery tests | The supported desktop/icon/MIME-link subset is reversible; system-wide integration remains rejected. |
| Transaction interruption | deterministic install failpoints and recovery regression tests | Known pkg transactions settle on the old or new committed state. |
| Runtime library selection | real ELF cross-format fixtures and scoped command views | Providers are selected by evidence; global loader paths are not exported. |
| MCP and structured CLI | JSON and MCP process tests | Machine-facing entry points use typed/structured results and preserve failures. |
| Garbage collection | reachability and foreign-content retention tests | Uncertain or foreign objects are retained. |

## Residual risks

- The runner is rootless but not a process sandbox; installed programs retain
  the user's normal authority.
- Host changes after verification can race with launch. Runtime fingerprints
  are revalidated where available, but external mutation cannot be made
  atomic with a process start.
- Architectures, musl, foreign loaders, native interpreter extensions and
  broad desktop/service integrations remain outside the verified matrix. The
  narrow user-space link subset is covered by the M4 tests above.
- Repository signature format and final store identity semantics are explicitly
  post-v1 under ADR-020.

The review found no blocker against the bounded v1 support claim. The residual
risks above must remain visible in the support matrix and release notes.
