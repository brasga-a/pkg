# Decision Ledger

| ID | Status | Decision | ADR | Milestone |
|---|---|---|---|---|
| DEC-001 | Accepted | Rust 2024 implementation | ADR-001 | M0 |
| DEC-002 | Accepted | Tokio for async network/process orchestration | ADR-002 | M2 |
| DEC-003 | Accepted | Pkg-owned isolated store | ADR-003 | M1 |
| DEC-004 | Accepted | Do not mutate native package DBs implicitly | ADR-004 | M1 |
| DEC-005 | Accepted | Format adapters normalize artifacts | ADR-005 | M1 |
| DEC-006 | Accepted | SQLite state database | ADR-006 | M1 |
| DEC-007 | Accepted | Repository metadata normalized into snapshots | ADR-007 | M2 |
| DEC-008 | Accepted | Trust evidence kept source-specific | ADR-008 | M2 |
| DEC-009 | Accepted | Solver behind normalized constraint IR | ADR-009 | M3 |
| DEC-010 | Accepted | Profiles expose binaries by links/wrappers | ADR-010 | M1 |
| DEC-011 | Accepted | Maintainer scripts default-deny | ADR-011 | M1 |
| DEC-012 | Accepted | Durable staged transactions and recovery | ADR-012 | M1 |
| DEC-013 | Accepted | Digest-addressed artifact cache | ADR-013 | M2 |
| DEC-014 | Accepted | Rootless user install is default | ADR-014 | M1 |
| DEC-015 | Accepted | Stable CLI nouns/verbs, internal API unstable | ADR-015 | M1 |
| DEC-016 | Accepted | Cross-distro mapping uses capabilities, not names alone | ADR-016 | M2 |
| DEC-017 | Accepted | Support order deb -> rpm -> ALPM | ADR-017 | M1-M3 |
| DEC-018 | Accepted | Host integration must be explicit/reversible | ADR-018 | M4 |
| DEC-019 | Deferred | Container fallback for non-relocatable packages | — | Post-v1 |
| DEC-020 | Deferred | Native host dependency provider | — | M3+ |
| DEC-021 | Open | Pkg-native repository signing format | — | M5 |
| DEC-022 | Open | Content-addressed store versus artifact-derived store IDs | — | M5 |
| DEC-023 | Accepted | Agent-first architecture and native MCP server | ADR-019 | M4-M5 |

## Rule

If this ledger and an accepted ADR disagree, implementation stops until the documentation defect is resolved.
