# Local State Database

SQLite is the default state engine.

## Core tables

- packages
- store_objects
- artifacts
- activations
- profiles
- transactions
- repositories
- repository_snapshots
- package_sources
- file_ownership (only for pkg-owned/exposed paths)
- trust_evidence

## Rules

- SQLite is metadata authority for pkg logical state.
- Store filesystem existence is separately verified during recovery.
- Foreign/native package-manager databases are queried only through optional read-only providers.
- Schema migrations are versioned and transactional.

## Locking

A process-level writer lock protects mutating CLI sessions. SQLite locking alone is not the full filesystem transaction lock.
