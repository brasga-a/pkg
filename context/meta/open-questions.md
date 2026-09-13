# Open Questions

OQ-001 — Should the final store ID be artifact-derived or a stronger normalized closure hash?

OQ-002 — Which Debian repository metadata/signature implementation should be supported first?

OQ-003 — What exact subset of RPM rich dependencies maps cleanly into the first solver IR?

OQ-004 — Should pkg query native package databases read-only to satisfy capabilities by default, or require opt-in?

OQ-005 — What is the policy for GLIBC symbol-version requirements?

OQ-006 — Should package-local shared libraries be injected via wrapper environment, patched RUNPATH, or only supported when upstream is naturally relocatable?

OQ-007 — Which desktop integrations are safe enough for v1?

OQ-008 — How are licenses surfaced before installation?

OQ-009 — What repository rollback/freeze protections are required for pkg-native repositories?

OQ-010 — When should packages be classified `ContainerPreferred` instead of `Unsupported`?

OQ-011 — How are config files handled if a future system-wide mode is added?

OQ-012 — Is a daemon ever necessary, or should pkg remain transaction-per-CLI invocation?
