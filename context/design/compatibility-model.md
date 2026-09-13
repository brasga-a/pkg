# Compatibility Model

## Why package-name mapping is insufficient

Examples such as Debian `libc6`, Fedora `glibc`, and Arch `glibc` refer to related runtime capability, but mapping names alone does not prove ABI compatibility.

pkg should reason about capabilities:

```text
package vocabulary
     ↓
capability candidates
     ↓
host/package evidence
```

## HostFacts

- kernel + version;
- architecture;
- libc family/version;
- dynamic linker path;
- available ELF SONAMEs;
- CPU feature policy if needed;
- filesystem features;
- desktop session;
- init system;
- native package manager presence.

## Compatibility result

Each requirement becomes:
- `SatisfiedByClosure`
- `SatisfiedByHost`
- `NeedsNativeProvider`
- `NeedsIntegration`
- `Unknown`
- `Unsatisfied`

Unknown is not success.

## ABI checks

For ELF payloads inspect:
- interpreter;
- DT_NEEDED libraries;
- RPATH/RUNPATH;
- symbol version requirements when practical;
- architecture/class/endianness.

Use binary inspection rather than `ldd` on untrusted executables, because executing/loading package binaries during inspection is unsafe.
