# Binary and Local Store Research

Homebrew's Cellar/keg/opt model and Nix's store/profile model illustrate an important separation:

```text
installed object != active command
```

pkg uses the same architectural family:

```text
store object
   |
profile activation
   |
PATH-visible command
```

Benefits:
- side-by-side versions;
- switch/rollback;
- deterministic uninstall;
- no mutable shared payload directory.

Trade-off:
- software with compiled absolute prefixes may not relocate.

The MVP must collect real-world compatibility data rather than assuming wrappers solve every application.
