# CLI Design

Temporary binary name: `pkg`.

## Initial commands

```bash
pkg install <name|path>
pkg remove <name>
pkg list
pkg info <name>
pkg search <query>
pkg repo sync
pkg update
pkg upgrade
pkg repo list
pkg repo add
pkg repo update # alias of pkg repo sync
pkg doctor
pkg gc
```

## Safety UX

Every install can expose:

```text
Source
Artifact digest
Package format
Architecture
Store destination
Dependencies
Host capabilities used
Scripts ignored
Host integrations
Activation conflicts
```

`--dry-run` prints the InstallPlan.

## Exit behavior

Machine-readable exit codes distinguish:
- package not found;
- incompatible host;
- verification failure;
- resolution failure;
- activation conflict;
- transaction/recovery failure.
