# Report — Repository / host release mismatch

## Incident

A host running **Ubuntu Resolute** had an Ubuntu **Noble (24.04)** repository configured in `pkg`.

`pkg` resolved and installed `libvirt-clients 10.0.0-2ubuntu8` from Noble. Its `virsh` executable was activated in the default `pkg` profile and therefore appeared before the native `/usr/bin/virsh` in `PATH`.

The host itself provided `virsh 12.0.0` and the newer libxml2 ABI. The Noble binary expected `libxml2.so.2`, so invoking `virsh` resolved to the pkg-managed binary and failed with:

```text
virsh: error while loading shared libraries: libxml2.so.2: cannot open shared object file: No such file or directory
```

This was not a corrupted package or an incorrect repository response. The repository suite was simply older than the host release.

## Root cause

The configured repository distribution was:

```toml
distribution = "noble"
```

while the host release was Ubuntu Resolute.

Because the pkg profile is intentionally placed before system locations in `PATH`, a successfully activated foreign/older executable can shadow the host executable with the same command name.

## Why this matters

Cross-distribution package management means repository/host mismatches can be intentional. Therefore, `pkg` should not blindly reject every repository that differs from the host.

However, same-distribution release mismatches are useful compatibility evidence and should not be silent. They can create ABI/SONAME incompatibilities even when package metadata and download integrity are valid.

The incident also demonstrates that repository provenance and runtime compatibility are separate checks:

```text
trusted repository
    !=
compatible package
```

## Proposal

### 1. Detect host release facts

Read `/etc/os-release` and normalize at least:

```text
ID
ID_LIKE
VERSION_ID
VERSION_CODENAME
```

Expose these as immutable host facts to repository and compatibility planning code rather than reading `/etc/os-release` ad hoc in CLI paths.

### 2. Record repository distro/suite explicitly

Repository configuration should preserve enough provenance to answer:

```text
repository family: ubuntu
suite: noble
host family: ubuntu
host suite: resolute
```

The existing `distribution` field should remain source semantics. Do not silently rewrite it.

### 3. Warn on same-family release mismatch

When adding or updating a repository whose distro family matches the host but whose suite differs, emit a clear warning:

```text
warning: repository release differs from host

Host:       ubuntu / resolute
Repository: ubuntu / noble

Packages from this repository may require libraries or ABIs that are not
available on this host.
```

A mismatch should be treated as compatibility evidence, not automatically as an error, because explicit cross-release use can be valid.

### 4. Require explicit acknowledgement for automatic/default repository setup

If `pkg` generates a default repository configuration, it should prefer the detected host release rather than a hard-coded suite.

For example, on Ubuntu:

```text
VERSION_CODENAME=resolute
```

should result in a Resolute repository configuration.

If a user intentionally selects another suite, an explicit mechanism can make the choice visible, for example:

```bash
pkg repo add ubuntu-noble ... --allow-release-mismatch
```

or a persisted configuration field such as:

```toml
allow_release_mismatch = true
```

The exact CLI/API should be decided separately; the invariant is that an implicit/default setup must not silently select a different release.

### 5. Keep ELF/runtime compatibility as the final gate

Repository release detection is advisory evidence. Installation should still independently inspect runtime requirements.

For the observed case:

```text
virsh
  DT_NEEDED -> libxml2.so.2

host
  provides -> libxml2.so.16
```

If `libxml2.so.2` is neither supplied by the package closure nor available through the effective host loader paths, activation should fail before the command becomes visible in the profile.

This remains important even when the repository suite matches the host, because third-party repositories can still ship incompatible binaries.

### 6. Surface provenance in planning/dry-run output

A dry-run should make release mismatch visible before mutation:

```text
Repository: ubuntu-noble
Host:       ubuntu-resolute
Compatibility: release mismatch (explicitly allowed)
```

This makes debugging substantially easier.

## Suggested implementation boundary

```text
/etc/os-release
      ↓
HostFacts
      ↓
Repository compatibility hint
      ↓
Install planner
      ↓
ELF / capability verification
      ↓
activation
```

Repository mismatch detection must not replace capability/ELF checks and must not become a distro-name-only compatibility model.

## Acceptance criteria

- `pkg` can identify the host distro ID and release codename from `/etc/os-release`.
- Default Ubuntu repository configuration uses the host codename instead of a hard-coded release.
- Adding/updating an Ubuntu repository for a different Ubuntu release produces an explicit warning or requires explicit acknowledgement, according to the final CLI decision.
- Foreign repositories remain possible; cross-distro use is not globally blocked.
- Dry-run/install planning exposes repository provenance and release mismatch state.
- An executable with an unresolved required SONAME is not activated solely because it came from a trusted repository.
- Tests cover at least a synthetic `ubuntu/resolute` host with an `ubuntu/noble` repository.

## Regression scenario

```text
Given:
  host ID=ubuntu
  host VERSION_CODENAME=resolute
  repository family=ubuntu
  repository distribution=noble

When:
  repository is added or synchronized

Then:
  pkg reports the release mismatch

And when:
  a package from that repository requires a SONAME unavailable on the host

Then:
  installation/activation fails with a compatibility error rather than exposing
  a broken command in the profile.
```

## Non-goals

This proposal does not imply:

- matching packages solely by distro name;
- rejecting all foreign repositories;
- treating Ubuntu codenames as a universal compatibility mechanism;
- replacing ELF, ABI, capability, or dependency verification with release checks.

The release comparison is an early diagnostic and policy signal; compatibility must still be proven by concrete package and host evidence.
