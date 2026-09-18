# Benchmark baseline — 2026-09-17

This is a reproducible smoke baseline for the checked-in `hello-world` Debian
fixture. It is evidence for the local host only; it is not a cross-distribution
performance promise.

## Environment

| Item | Value |
|---|---|
| Host | Linux 7.0.0-31-generic x86_64 |
| CPU | AMD Ryzen 5 1600 Six-Core Processor |
| Rust | rustc 1.98.1 (48a229cea 2026-09-01) |
| Runner | hyperfine 1.20.0 |
| Build | `cargo build --release` |
| Runs | 10 timed, 2 warmups |

Command used:

```text
benchmarks/run-gates.sh \
  --runs 10 --warmup 2 --output-dir /tmp/pkg-bench-gates
```

## Results

| Operation | Mean | Standard deviation | Min–max | Exit codes |
|---|---:|---:|---:|---|
| `repository-parsing` (500 Debian stanzas) | 4.09 ms | 1.24 ms | 2.18–5.59 ms | all 0 |
| `solving` (`hello-world` local plan) | 4.97 ms | 1.50 ms | 3.26–8.70 ms | all 0 |
| `extraction` (`hello-world` payload) | 3.56 ms | 1.02 ms | 1.70–4.92 ms | all 0 |
| `activation` (fresh install and profile publication) | 28.90 ms | 4.22 ms | 23.22–34.64 ms | all 0 |
| `recovery` (one incomplete transaction) | 7.93 ms | 2.15 ms | 5.90–11.76 ms | all 0 |

The gate driver is [`benchmarks/run-gates.sh`](../benchmarks/run-gates.sh). It
uses the release `bench_gates` example, fresh temporary roots for mutating
operations, and the checked-in `hello-world` artifact. The measurements include
process startup and filesystem effects; they are baselines for regression
detection on this host, not cross-machine performance promises. Raw hyperfine
output is kept in [`benchmarks/results/gates/results.json`](../benchmarks/results/gates/results.json).
