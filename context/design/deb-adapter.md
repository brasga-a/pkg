# Debian `.deb` Adapter

A `.deb` is an `ar` container typically holding:
- `debian-binary`;
- `control.tar.*`;
- `data.tar.*`.

The adapter parses `control` fields such as Package, Version, Architecture, Depends, Pre-Depends, Provides, Conflicts, Breaks, Replaces and scripts from the control archive.

## Policy

- Do not call `dpkg` for parsing.
- Do not run `preinst`, `postinst`, `prerm`, `postrm`, triggers or `debconf`.
- Treat `Depends` package names as Debian vocabulary until mapped to capabilities.
- Preserve alternatives (`a | b`) and architecture/version qualifiers.
- Reject packages whose functionality clearly depends on mandatory lifecycle scripts unless an explicit integration adapter exists.

## Extraction

Only `data.tar.*` payload is materialized, into staging. Absolute package paths become paths relative to the store root; traversal and escaping links are rejected.
