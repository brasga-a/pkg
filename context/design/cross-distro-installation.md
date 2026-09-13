# Cross-Distro Installation

## Core principle

Source formats are **artifact formats**, not licenses to reproduce source-distro lifecycle semantics on another host.

### Debian package on Fedora/Arch

Safe path:

```text
.deb
 -> parse
 -> extract to staging
 -> inspect
 -> map requirements to capabilities
 -> verify host/package closure
 -> store
 -> activate supported integrations
```

Unsafe path:

```text
dpkg --force-all package.deb
```

or raw extraction into `/`.

## Adaptation levels

### Level 0 — no adaptation
Statically/self-contained binary.

### Level 1 — path activation
Package payload works from store; only binary links are required.

### Level 2 — deterministic relocation
Known text/config paths or wrapper environment can safely redirect lookup paths.

### Level 3 — host integration
Desktop entry, icon, MIME, service or other explicit integration adapter.

### Level 4 — source-distro runtime required
Use future container fallback or reject.

## System packages

If a package owns canonical paths under `/lib`, `/usr/lib`, `/etc/pam.d`, boot/init/kernel/package-manager state, default policy rejects cross-installation.
