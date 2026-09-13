# Filesystem Layout and Relocation

Distribution packages usually encode paths relative to `/`.

pkg remaps payload root to a store object:

```text
/usr/bin/foo
/usr/share/foo/data
```

becomes:

```text
<store>/usr/bin/foo
<store>/usr/share/foo/data
```

This alone does not make software relocatable. Applications may contain compiled absolute paths or expect FHS locations.

## Inspection

Identify:
- ELF interpreter and RPATH/RUNPATH;
- symlink targets;
- absolute paths in known text metadata;
- desktop entries;
- pkg-config files;
- systemd units;
- shared libraries.

## Policy

No broad binary patching in MVP. Deterministic format-specific relocation may be added only with tests and an ADR.
