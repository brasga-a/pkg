# pkg Benchmarking Suite

This directory contains performance benchmarks for `pkg`.

There are two distinct suites:

```text
benchmarks/
├── run.sh                # local .deb install/info and optional dpkg comparison
├── cli_commands.py       # CLI/catalog benchmark for 0.1.0-alpha.2 commands
├── fallback_bench.py
├── plot.py
└── results/
```

The two suites answer different questions and their numbers should not be mixed into one speedup claim.

## 1. Local install benchmark

The original runner measures the local artifact path:

```bash
./benchmarks/run.sh
./benchmarks/run.sh ~/Downloads/discord.deb
./benchmarks/run.sh ~/Downloads/discord.deb --compare-dpkg
```

It is useful for measuring the end-to-end local `.deb` path and, optionally, comparing it with `dpkg -i`.

## 2. CLI/catalog benchmark — 0.1.0-alpha.2

`cli_commands.py` exercises the command paths introduced or expanded in `0.1.0-alpha.2`:

```text
pkg list                    # empty profile
pkg list                    # installed profile
pkg remove
pkg remove --dry-run
pkg repo list               # no repositories
pkg repo add
pkg repo list               # configured repository
pkg update
pkg search                  # hit
pkg search                  # miss
pkg install <remote> --dry-run
```

Run it with the bundled fixture:

```bash
python3 benchmarks/cli_commands.py
```

Or with a real local `.deb` for the installed/remove baselines:

```bash
python3 benchmarks/cli_commands.py ~/Downloads/discord.deb \
  --runs 50 \
  --warmup 10 \
  --catalog-size 5000
```

Options:

```text
--runs N           measured samples per command (default: 30)
--warmup N         discarded warmup samples (default: 5)
--catalog-size N   number of fake remote packages in the deterministic catalog (default: 1000)
--profile NAME     profile used by installed/list/remove cases
--output-dir DIR   result root (default: benchmarks/results)
```

## Deterministic local repository

The CLI/catalog suite does **not** use Debian or Ubuntu mirrors during timing.

It creates a Debian-style repository fixture in a temporary directory and serves it over `127.0.0.1` using Python's HTTP server. The fixture contains:

```text
dists/bench/InRelease
dists/bench/main/binary-amd64/Packages.gz
```

`Packages.gz` contains the requested `--catalog-size` number of synthetic package records, including a known package called `bench-target`.

This makes `pkg update` include the actual alpha.2 path:

```text
HTTP fetch
  -> InRelease
  -> Packages.xz probe / Packages.gz fetch
  -> decompression
  -> package metadata parsing
  -> SQLite snapshot commit
```

while avoiding public-network latency and mirror variance.

The generated catalog baseline is then reused for `search` and remote `install --dry-run`, so those tests measure local catalog/query behavior rather than synchronization.

## Reproducible state

Each measured command starts from a dedicated logical baseline copied outside the timed interval:

```text
empty baseline
  -> list empty
  -> repo list empty
  -> repo add

installed baseline
  -> list installed
  -> remove
  -> remove --dry-run

repo-configured baseline
  -> repo list configured
  -> update

catalog baseline
  -> search hit
  -> search miss
  -> install remote --dry-run
```

The benchmark never uses the developer's normal `~/.local/share/pkg` state.

## Results

Each run creates a timestamped directory:

```text
benchmarks/results/<timestamp>-cli-alpha2/
├── results.json
├── results.md
└── environment.txt
```

The report records, for each command:

- mean;
- standard deviation;
- median;
- p95;
- minimum/maximum;
- raw samples.

`environment.txt` records the commit, branch, artifact digest, catalog size, CPU/kernel information, run count, warmup count and local repository endpoint.

## Interpreting the numbers

Do not compare `pkg search` directly with `pkg remove` and call one “faster”; they perform different work.

The intended use is regression tracking by row:

```text
pkg search (hit)
commit A: 2.1 ms
commit B: 2.0 ms
commit C: 5.7 ms  <- investigate
```

For `pkg update`, always report `--catalog-size` because update cost should be interpreted as a function of catalog size.

External comparisons against `apt update`, `apt-cache search`, or other package managers should be a separate benchmark with equivalent repository datasets and clearly documented semantic differences.
