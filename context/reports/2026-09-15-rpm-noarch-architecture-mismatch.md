# RPM `noarch` architecture mismatch

Date: **2026-09-15**  
Status: **confirmed bug**  
Affected branch: **0.1.0-beta.1**  
Area: **architecture normalization / RPM compatibility**

## Summary

Installing an architecture-independent RPM package fails on an `x86_64` host because RPM's `noarch` architecture is not normalized as architecture-independent by `pkg`.

Observed command:

```text
pkg install rpmdevtools
```

Observed result:

```text
Resolving package target 'rpmdevtools'...
Found rpmdevtools 9.6-8.fc41 [rpm] in fedora-41
Downloading rpmdevtools...
Error installing 'rpmdevtools': Architecture mismatch: package is 'noarch', host is 'x86_64'
Error: Installation failed for requested target(s).
```

`rpmdevtools` is legitimately published as `noarch`. In RPM terminology, `noarch` means that the package payload is architecture-independent and is therefore compatible with an `x86_64` host unless some separate runtime constraint makes it incompatible.

The current rejection is therefore a false architecture mismatch.

## Root cause

The RPM adapter reads the architecture from RPM metadata and forwards it to the shared architecture parser:

```rust
let raw_arch = pkg.metadata.get_arch().unwrap_or("x86_64");
let architecture = Architecture::parse(raw_arch);
```

The shared parser currently recognizes:

```rust
"amd64" | "x86_64" | "x86-64" => Self::X86_64,
"all" => Self::All,
"any" => Self::Any,
other => Self::Other(other.to_string()),
```

Because `noarch` is not recognized, it becomes:

```rust
Architecture::Other("noarch".to_string())
```

`matches_host()` only treats `All` and `Any` as architecture-independent:

```rust
(Self::All | Self::Any, _) => true,
(Self::X86_64, Self::X86_64) => true,
(Self::Other(a), Self::Other(b)) => a == b,
_ => false,
```

As a result:

```text
Other("noarch") != X86_64
```

and the planner emits `ArchitectureMismatch`.

## Expected behavior

RPM `noarch`, Debian `all`, and equivalent architecture-independent package markers must normalize to the same semantic representation.

For example:

```text
RPM:     noarch  ─┐
Debian:  all     ─┼─> Architecture::All
Generic: any     ─┘   or another explicit architecture-independent variant
```

Then:

```rust
Architecture::parse("noarch").matches_host(&Architecture::X86_64)
```

must return:

```rust
true
```

## Recommended fix

The minimal safe fix is to normalize RPM `noarch` in `Architecture::parse()`:

```rust
pub fn parse(s: &str) -> Self {
    match s.trim().to_lowercase().as_str() {
        "amd64" | "x86_64" | "x86-64" => Self::X86_64,
        "all" | "noarch" => Self::All,
        "any" => Self::Any,
        other => Self::Other(other.to_string()),
    }
}
```

This is preferable to special-casing `noarch` inside `RpmAdapter`, because architecture semantics belong in the normalized domain model rather than in a format-specific compatibility check.

A future cleanup could rename `Architecture::All` to a format-neutral variant such as `Architecture::Independent`, but that is not required to fix the bug.

## Required tests

Add unit coverage for the normalized architecture model:

```rust
#[test]
fn rpm_noarch_matches_x86_64_host() {
    let package = Architecture::parse("noarch");
    let host = Architecture::X86_64;

    assert!(package.matches_host(&host));
}
```

Also verify aliases remain correct:

```rust
assert!(Architecture::parse("all").matches_host(&Architecture::X86_64));
assert!(Architecture::parse("any").matches_host(&Architecture::X86_64));
assert!(Architecture::parse("x86_64").matches_host(&Architecture::X86_64));
assert!(Architecture::parse("amd64").matches_host(&Architecture::X86_64));
```

Add a negative case to ensure the fix does not weaken real architecture rejection:

```rust
assert!(!Architecture::parse("aarch64").matches_host(&Architecture::X86_64));
```

Finally, add an RPM integration fixture or repository-level test proving that a real `noarch` package reaches the next installation stage instead of failing at the architecture gate.

## Acceptance criteria

The bug is fixed when all of the following hold:

- `Architecture::parse("noarch")` produces an architecture-independent normalized value;
- a `noarch` RPM is accepted on an `x86_64` host by the architecture gate;
- an actual incompatible architecture such as `aarch64` remains rejected on `x86_64`;
- Debian `all` behavior remains unchanged;
- the RPM adapter does not need package-format-specific bypass logic;
- `pkg install rpmdevtools` no longer fails with `Architecture mismatch: package is 'noarch', host is 'x86_64'`.

## Severity and impact

Severity: **medium-high for RPM support**.

This is not a cosmetic edge case. `noarch` is a standard RPM architecture class and is common for packages containing scripts, Python modules, metadata, fonts, configuration, documentation, and other architecture-independent payloads. Leaving this unresolved causes valid Fedora/RPM packages to be rejected systematically and makes RPM repository support appear less compatible than it actually is.

The fix itself is small, but it closes a fundamental normalization gap in the cross-distribution architecture model.
