# Benchmarks and Evidence Gates

Performance is secondary to correctness in early milestones, but measurable gates prevent pathological design.

## Repository
- sync index parsing throughput;
- peak memory for 10k/100k/500k candidates;
- incremental refresh time.

## Solver
- candidate graph sizes;
- satisfiable solve latency;
- conflict explanation latency;
- worst-case fixture corpus.

## Install
- extraction throughput;
- peak memory during archive processing;
- activation switch latency;
- SQLite transaction latency.

## Correctness benchmarks
- 1,000 interrupted-install fault injection points;
- concurrent writer attempts;
- hostile archive corpus;
- dependency conflict corpus;
- ELF compatibility corpus.

Promotion requires reproducible tests, not anecdotal success on one package.
