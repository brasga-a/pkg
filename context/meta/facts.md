# Facts and Evidence Ledger

## F-001 — Distribution package formats contain lifecycle semantics
Debian packages can contain dependency relationships and installation/removal scripts. RPM includes Requires/Provides and scriptlets. ALPM packages may include install scripts.

Implication: archive extraction alone is not equivalent to native installation.

## F-002 — Native package managers track local ownership/state
dpkg/RPM/libalpm maintain installed-package metadata.

Implication: pkg cannot safely write foreign payloads into native paths while leaving native ownership unaware.

## F-003 — Repository metadata can be consumed without mirroring all payloads
APT-style, RPM metadata and ALPM repository databases separate searchable metadata from package payload retrieval.

Implication: pkg can build a normalized local catalog and download artifacts on demand.

## F-004 — Nix/Homebrew demonstrate side-by-side store/prefix models
Versioned install roots and activation links reduce overwrite conflicts.

Implication: a pkg-owned store is a proven architectural family, though pkg does not inherit all guarantees of either system.

## F-005 — RPM capability dependencies are richer than package-name dependencies
Requires/Provides may represent shared libraries and virtual features.

Implication: pkg's normalized model must support capabilities, not only package names.

## F-006 — Foreign scripts can depend on distro-specific helpers
Implication: maintainer scripts must be parsed as untrusted behavior, not executed by default.

## F-007 — ELF inspection can reveal interpreter, needed libraries and run paths without executing the binary
Implication: compatibility planning should use static binary parsing.
