# Linux Package Manager Ecosystem

## APT + dpkg

`dpkg` is the low-level Debian package database/installer. APT adds repository acquisition, dependency resolution and user-facing package operations.

Important architectural observations:
- `.deb` includes payload + control metadata + lifecycle scripts;
- repository metadata allows candidate discovery without downloading every `.deb`;
- dependencies use Debian package names, alternatives and Debian version semantics.

## DNF + RPM

RPM is the package format/database and transaction substrate. DNF/libdnf provide repository and dependency-management behavior.

Important observations:
- RPM has rich `Requires`/`Provides` capability semantics;
- metadata can represent shared-library capabilities;
- scriptlets and triggers participate in native lifecycle;
- repository metadata is separate from payloads.

## pacman + libalpm

pacman is the CLI frontend to libalpm.

Important observations:
- `.pkg.tar.*` is a tar-based package with ALPM metadata;
- sync databases describe repository package metadata;
- local libalpm database tracks installed state;
- package install scripts exist.

## Homebrew

Homebrew demonstrates a versioned Cellar/keg model and activation through links into a prefix. This strongly informs pkg's store/profile separation.

## Nix

Nix demonstrates unique store paths, profile generations, atomic switching and reachability-based garbage collection. pkg borrows the structural idea without claiming derivation-level reproducibility.

## Flatpak/AppImage/container tools

These solve a different portability layer:
- Flatpak uses runtimes/sandbox/application bundles;
- AppImage emphasizes self-contained portable app images;
- containers reproduce userspace environments.

pkg can later interoperate with these, but its initial purpose is package artifact ingestion and controlled host activation.
