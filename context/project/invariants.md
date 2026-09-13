# Architectural Invariants

INV-001 — pkg never silently mutates `dpkg`, RPM or libalpm installed-package databases.

INV-002 — foreign package payloads are not directly extracted into `/`.

INV-003 — package lifecycle scripts are default-deny until a dedicated policy classifies them.

INV-004 — archive extraction rejects absolute paths, `..` traversal and symlink/hardlink escapes.

INV-005 — every promoted store object has a verified artifact identity and normalized metadata record.

INV-006 — store content and activation state are separate concepts.

INV-007 — package name equality does not imply capability equality across distributions.

INV-008 — source-distro version syntax is preserved; cross-ecosystem comparisons occur only through explicitly normalized constraints.

INV-009 — libc/ELF ABI compatibility is checked by capability/evidence, not package-name mapping alone.

INV-010 — install planning is side-effect free.

INV-011 — only the transaction executor mutates package state.

INV-012 — one writer lock protects mutable pkg state.

INV-013 — interrupted transactions are detectable on next startup.

INV-014 — removal is ownership-based.

INV-015 — activation conflict is explicit; pkg never silently replaces an unrelated executable.

INV-016 — repository metadata freshness and package authenticity are separate trust dimensions.

INV-017 — cache entries are addressed by validated identity/digest, not only URL.

INV-018 — unsupported host integration fails closed.

INV-019 — system-critical packages are rejected in default policy.

INV-020 — dependency solving errors include an explanation chain suitable for users and tests.
