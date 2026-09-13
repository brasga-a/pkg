# `pkg info`

Show detailed package information.

## Syntax

```bash
pkg info <package>
```

The command may resolve either:
- an installed package;
- a repository candidate;
- a local artifact when a path is supplied.

Examples:

```bash
pkg info ripgrep
pkg info ./vendor.deb
```

## Suggested output sections

```text
Identity
  Name
  Version
  Architecture
  Source format

Provenance
  Repository
  Artifact URL
  Digest
  Trust result

Compatibility
  Package class
  Host capabilities
  Unsatisfied requirements
  Maintainer scripts

Local state
  Store object
  Active profile
  Installed date
  Reverse dependencies

Provides
  Commands
  Libraries/capabilities
```

## Why this command matters

`pkg info` is the audit surface for cross-distro decisions. The user should be able to understand *why* pkg accepted or rejected a package without reading internal logs.

## Network behavior

If the package is not installed and repository metadata is requested, use the latest active local repository snapshot. No implicit refresh is required.
