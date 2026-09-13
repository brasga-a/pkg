# Architecture Options

This document preserves alternatives considered. Canonical decisions live in ADRs and `design/`.

## AO-01 — Native frontend

Delegate to `apt`, `dnf` or `pacman` depending on host.

**Pros:** mature solvers and lifecycle handling.  
**Cons:** does not satisfy foreign-artifact installation; behavior remains distro-specific; cannot safely install `.deb` on Arch by delegation.

Rejected as the core architecture. It may later exist as an optional provider for satisfying host capabilities.

## AO-02 — Convert foreign package into host-native package

Examples conceptually resemble `.deb` -> `.rpm`.

**Pros:** host database retains ownership.  
**Cons:** metadata semantics, scripts, dependency names, versions and policies do not translate losslessly. Conversion can create a native-looking artifact with non-native assumptions.

Rejected as automatic default.

## AO-03 — Direct foreign install into `/`

Extract foreign payload and write files to host paths.

**Pros:** closest to native package layout.  
**Cons:** file ownership conflicts, maintainer scripts, ABI assumptions, irreversible interaction with native manager.

Rejected.

## AO-04 — Isolated pkg store + activation links

Materialize packages under a pkg-controlled store, expose selected binaries and integrations.

**Pros:** coexists with native package manager, side-by-side versions, deterministic removal, rollback-friendly, minimizes host mutation.  
**Cons:** relocation may fail; desktop/services/config integration need explicit adapters; dynamic libraries may still depend on host capabilities.

Accepted foundation.

## AO-05 — Container per application

Run foreign package in distro-matching container.

**Pros:** highest semantic compatibility with source distro.  
**Cons:** integration, storage, startup and GUI/device complexity; not a natural package-manager experience.

Deferred as an escape hatch for packages that cannot be safely materialized.

## AO-06 — Nix-like fully hermetic closure

Build or rewrite dependency closures into content-addressed immutable store.

**Pros:** reproducibility and isolation.  
**Cons:** requires package recipes/build graph, relocation/patching and ecosystem-scale metadata.

Inspirational, but beyond MVP. pkg borrows store/profile/GC concepts without claiming Nix-level reproducibility.
