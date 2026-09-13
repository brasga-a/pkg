# Security Architecture

## Threats

- malicious archive paths;
- decompression bombs;
- malicious symlink/hardlink graphs;
- package metadata memory exhaustion;
- repository rollback/freeze;
- compromised mirror;
- arbitrary maintainer scripts;
- poisoned executable activation;
- binary/library mismatch;
- TOCTOU during transaction;
- concurrent writers.

## Controls

- bounded parsing/extraction;
- no script execution;
- staging directories with restrictive permissions;
- same-filesystem atomic promotion;
- cryptographic digest validation;
- explicit repository trust;
- no shell evaluation;
- normalized path validation;
- writer lock;
- rootless default;
- default-deny system package policy.

## Security non-claim

An isolated filesystem prefix is not a sandbox. Installed executables run with the user's normal authority unless a future execution sandbox is explicitly added.
