# `pkg integrate` and `pkg deintegrate`

These commands explicitly expose or remove supported desktop entries, icons and
MIME package descriptions in the user's XDG data directory. They do not run
maintainer scripts, update system databases or write system-wide paths.

```bash
pkg integrate <package>
pkg integrate <package> --dry-run
pkg deintegrate <package>
```

Every created link is recorded with its package, store object, source digest,
kind and target path. Conflicts with an existing file or a user-replaced link
fail closed. `pkg remove` performs the same ownership check and reverses the
integration before collecting the package payload.
