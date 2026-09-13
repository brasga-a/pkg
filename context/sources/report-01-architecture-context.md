# Source Report 01 — Initial Architecture Context

## Goal

Design a Rust package manager that provides one installation interface across Linux distributions while respecting distribution-specific package semantics.

## Central finding

The largest mistake would be treating `.deb`, `.rpm` and `.pkg.tar.zst` as interchangeable archive formats and directly installing them into the host root.

They contain:
- dependency semantics;
- package-manager ownership assumptions;
- lifecycle scripts;
- distribution policy;
- ABI expectations.

## Recommended foundation

```text
foreign/native artifact
  -> adapter
  -> normalized metadata
  -> compatibility plan
  -> isolated pkg store
  -> activation profile
```

This architecture uses ideas proven by store/prefix-based systems while retaining the ability to consume existing distribution repositories.

## Key trade-off

Isolation prevents file-ownership conflict but does not guarantee relocatability. Real package compatibility must be empirically measured and classified.

## MVP

Start local and narrow:
- `.deb`;
- CLI/self-contained application;
- no scripts;
- rootless store;
- state DB;
- binary activation;
- deterministic uninstall.

Expand only after this vertical slice survives hostile archive, interruption and ABI compatibility tests.
