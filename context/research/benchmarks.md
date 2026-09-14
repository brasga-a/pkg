# Benchmarks and Evidence Gates

Performance is secondary to correctness in early milestones, but measurable gates prevent pathological design and make regressions visible.

## Benchmark classes

pkg uses four distinct benchmark classes. Results from one class must not be presented as if they measured another.

### Micro / engine

Measures isolated implementation primitives such as:

- archive metadata parsing;
- extraction throughput;
- ELF inspection;
- hashing;
- SQLite operations;
- activation-link creation.

These benchmarks are appropriate for optimizing implementation details because the compared work can be made semantically equivalent.

### Component

Measures a complete internal subsystem, for example:

- artifact -> normalized package;
- repository metadata -> normalized snapshot;
- dependency graph -> resolution result;
- install plan -> staged store object.

### End-to-end

Measures user-visible command paths such as `pkg install` or `pkg remove`.

A comparison such as `pkg install file.deb` versus `dpkg -i file.deb` is an **end-to-end installation-path comparison**, not a pure engine benchmark. The tools intentionally perform different work: pkg uses a rootless isolated store and default-denies maintainer scripts, while dpkg owns native system paths/state and executes Debian lifecycle semantics.

Any published relative speedup must state this semantic difference explicitly.

### Correctness / resilience

Performance numbers do not replace correctness evidence. Maintain dedicated corpora for:

- interrupted-install fault injection;
- concurrent writer attempts;
- hostile/malformed archives;
- dependency conflicts;
- ELF/ABI compatibility;
- transaction recovery.

## Reproducible install-state policy

Every timed installation begins from the same **logical package-manager state**.

For pkg:

- use a throwaway `--data-dir`;
- create one empty initialized baseline state;
- restore that baseline before every timed run;
- never benchmark against the developer's normal `~/.local/share/pkg` state.

For dpkg comparisons:

- use a disposable VM/container host whenever practical;
- refuse to benchmark a package already registered on the host;
- purge only the benchmark-installed package between runs;
- keep preparation outside the measured command.

Each tool receives its own preparation step. Do not remove/reset pkg as part of dpkg preparation or vice versa.

## Warm vs cold cache

Cache state and package-manager state are separate variables.

### Warm

Default mode. Every run starts from a clean logical install state, but Linux page cache is allowed to remain warm. This is representative of repeated development/desktop operations and produces lower variance.

### Cold

Before every measured run, synchronize writes and drop Linux page cache. This requires root and affects the whole machine, so it should be run only on an otherwise idle/disposable environment.

Warm and cold numbers must never be merged into one average.

## Required measurements

Latency report:

- mean;
- standard deviation;
- median;
- p95;
- minimum/maximum;
- raw per-run samples.

Resource report where GNU `/usr/bin/time -v` is available:

- wall time;
- user CPU time;
- system CPU time;
- peak RSS;
- filesystem input/output operation counters;
- major/minor page faults;
- voluntary/involuntary context switches.

For repository/resolver milestones also track:

- metadata parsing throughput;
- candidate graph size;
- constraints evaluated;
- peak memory;
- incremental refresh latency.

## Environment evidence

Every benchmark run intended for comparison/publication must record:

- UTC timestamp;
- git commit;
- target artifact SHA-256 and byte size;
- distro/kernel;
- CPU model/count and governor where available;
- Rust/Cargo/pkg versions;
- hyperfine/dpkg versions where applicable;
- cache mode;
- run/warmup counts;
- CPU affinity if pinned.

## Package corpus

Do not infer universal performance from one package. Maintain representative classes:

1. tiny/self-contained fixture;
2. normal dynamically-linked CLI;
3. dependency/metadata-heavy CLI or application;
4. large desktop application.

Discord is useful as a real desktop artifact, but it is one corpus member, not the benchmark definition.

## Comparison policy

The initial performance gate is regression-oriented, not "pkg must beat dpkg".

A valid external comparison requires:

- same artifact where possible;
- same machine/filesystem;
- explicit warm/cold mode;
- identical logical starting state per tool;
- preparation excluded from timing;
- failures excluded rather than timed as successful samples;
- environment metadata published with results;
- semantic differences documented.

The architecture may justify modest overhead if it buys safe cross-distro behavior. Performance claims must therefore report absolute metrics alongside any relative ratio.

Promotion requires reproducible evidence, not anecdotal success on one package or one screenshot.
