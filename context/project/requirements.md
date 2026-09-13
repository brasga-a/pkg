# Requirements

## Functional

### REQ-001 Artifact ingestion
pkg must identify and parse supported artifact formats without invoking the host native package manager.

### REQ-002 Normalization
Adapters must map package metadata into a format-neutral internal model while preserving original metadata for audit.

### REQ-003 Compatibility planning
Before mutation, pkg must produce an install plan containing architecture, dependency/capability, conflict and integration findings.

### REQ-004 Isolated installation
Payloads must be installed under pkg-owned store roots.

### REQ-005 Deterministic ownership
Every pkg-created path must be attributable to an installed object or activation record.

### REQ-006 Removal
Removal must never delete a path merely because it matches a package payload path; it deletes only owned store/activation state.

### REQ-007 Version coexistence
Multiple versions may exist in the store; activation selects at most one provider of a command in a profile.

### REQ-008 Repository catalog
Remote package metadata must be syncable independently of payload download.

### REQ-009 Integrity
Payload digest verification must occur before install promotion.

### REQ-010 Trust
Signature verification and trust policy must be explicit. “Downloaded over HTTPS” is not equivalent to package authenticity.

### REQ-011 Transactions
Install/update/remove operations must be recoverable from interruption.

### REQ-012 Rootless default
Normal application installation must work without root when package behavior permits.

## Non-functional

- bounded archive extraction;
- no path traversal;
- clear solver explanations;
- structured diagnostics;
- no shell evaluation of metadata;
- concurrency-safe state;
- deterministic dry-run;
- auditable source and digest;
- measurable install/sync performance.

## Compatibility claims

A package is not declared supported solely because its archive format is supported. Support is a tuple of:

```text
format + package class + architecture + host capabilities + integration mode
```
