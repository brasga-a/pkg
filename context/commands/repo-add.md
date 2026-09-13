# `pkg repo add`

Add a repository configuration.

## Syntax

Provisional:

```bash
pkg repo add <id> <url> --type <type>
```

Examples:

```bash
pkg repo add vendor https://packages.example.org/debian --type debian
pkg repo add fedora https://example.org/repo --type rpm
```

Exact repository-specific arguments may evolve.

## Responsibilities

`repo add` creates configuration. It does not implicitly trust arbitrary signing keys provided by the repository payload.

The command must validate:
- repository ID uniqueness;
- supported URL scheme;
- supported repository adapter;
- conflicting configuration;
- explicit trust configuration.

## Trust roots

Adding a repository and trusting its signing root are separate security concepts.

Future UX may combine them only when the root fingerprint is shown and explicit user confirmation/policy is recorded.

## Initial sync

Recommended default:
- write validated config;
- optionally offer/perform a first metadata refresh depending on final CLI policy.

The repository must not become `ready` until a valid snapshot exists.

## Files

Repository configuration is expected under:

```text
~/.config/pkg/repos.d/<id>.toml
```

or equivalent selected configuration root.
