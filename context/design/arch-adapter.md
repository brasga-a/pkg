# ALPM `.pkg.tar.zst` Adapter

Arch packages are tar archives containing metadata such as `.PKGINFO`, `.BUILDINFO`, `.MTREE` and optional `.INSTALL`.

Normalize:
- pkgname/pkgver/arch;
- depend/optdepend/provides/conflict/replaces;
- file list;
- install script presence;
- package signature provenance when available from repository context.

## Policy

`.INSTALL` is data, not executable authority.

ALPM repository metadata and package payload are distinct inputs; repository sync may know package dependencies without downloading package archives.
