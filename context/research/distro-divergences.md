# Debian, Fedora/RPM and Arch Divergences

## Dependency vocabulary

The same conceptual library may have different package names. Names are policy-level labels, not ABI proof.

## Version syntax

Debian, RPM and ALPM version comparison rules differ. Do not coerce every version into SemVer.

## Lifecycle

Debian maintainer scripts, RPM scriptlets/triggers and ALPM install scripts have different conventions and helper assumptions.

## Filesystem/policy

Distributions differ in:
- package splitting;
- library locations and compatibility symlinks;
- service presets;
- user/group creation policy;
- SELinux integration;
- config ownership;
- triggers/cache refresh behavior.

## ABI

Even when a library exists under a familiar name, binaries may require:
- a particular ELF interpreter;
- a specific SONAME;
- versioned symbols;
- kernel features.

## Recommended normalization axis

Normalize **capabilities and evidence**, not distro package names.

Example conceptual mapping:

```text
Debian libc6 ─┐
RPM glibc ────┼──> capability: glibc ABI/version evidence
Arch glibc ───┘
```

The mapping proposes what to inspect; it does not itself satisfy the requirement.
