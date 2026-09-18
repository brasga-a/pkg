#!/usr/bin/env python3
"""Fallback benchmark runner used when hyperfine is unavailable.

The runner deliberately treats command/prepare failures as benchmark failures instead
of recording their runtime as a successful sample.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import time


def run_checked(cmd: str, *, timed: bool) -> float:
    start = time.perf_counter()
    result = subprocess.run(
        ["/bin/bash", "-lc", cmd],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    duration = time.perf_counter() - start
    if result.returncode != 0:
        kind = "benchmark command" if timed else "prepare command"
        raise RuntimeError(f"{kind} failed with exit code {result.returncode}: {cmd}")
    return duration


def percentile(values: list[float], p: float) -> float:
    ordered = sorted(values)
    index = max(0, math.ceil(p * len(ordered)) - 1)
    return ordered[index]


def calculate_stats(times: list[float]) -> dict:
    n = len(times)
    mean = sum(times) / n
    variance = sum((x - mean) ** 2 for x in times) / (n - 1) if n > 1 else 0.0
    ordered = sorted(times)
    median = (
        ordered[n // 2]
        if n % 2
        else (ordered[n // 2 - 1] + ordered[n // 2]) / 2.0
    )
    return {
        "mean": mean,
        "stddev": math.sqrt(variance),
        "median": median,
        "p95": percentile(times, 0.95),
        "min": min(times),
        "max": max(times),
        "times": times,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="pkg fallback benchmark runner")
    parser.add_argument("--command", "-c", action="append", required=True)
    parser.add_argument("--prepare", "-p", action="append", default=[])
    parser.add_argument("--name", action="append", default=[])
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--comparable", action="store_true")
    parser.add_argument("--export-json")
    parser.add_argument("--export-markdown")
    args = parser.parse_args()

    if args.runs < 1 or args.warmup < 0:
        parser.error("--runs must be >= 1 and --warmup must be >= 0")
    if len(args.prepare) > len(args.command):
        parser.error("more --prepare values than commands")
    if len(args.name) > len(args.command):
        parser.error("more --name values than commands")

    results = []
    print(f"\nRunning benchmark ({args.runs} runs, {args.warmup} warmup rounds)...")

    for idx, cmd in enumerate(args.command):
        prep = args.prepare[idx] if idx < len(args.prepare) else None
        name = args.name[idx] if idx < len(args.name) else cmd
        print(f"\nBenchmark {idx + 1}/{len(args.command)}: {name}")

        for _ in range(args.warmup):
            if prep:
                run_checked(prep, timed=False)
            run_checked(cmd, timed=True)

        times: list[float] = []
        for _ in range(args.runs):
            if prep:
                run_checked(prep, timed=False)
            times.append(run_checked(cmd, timed=True))

        stats = calculate_stats(times)
        stats["command"] = name
        stats["raw_command"] = cmd
        results.append(stats)
        print(
            "  mean={:.2f} ms  median={:.2f} ms  p95={:.2f} ms  stddev={:.2f} ms".format(
                stats["mean"] * 1000,
                stats["median"] * 1000,
                stats["p95"] * 1000,
                stats["stddev"] * 1000,
            )
        )

    fastest = min(results, key=lambda item: item["mean"]) if args.comparable else None

    if args.export_json:
        os.makedirs(os.path.dirname(os.path.abspath(args.export_json)), exist_ok=True)
        with open(args.export_json, "w", encoding="utf-8") as handle:
            json.dump({"results": results}, handle, indent=2)

    if args.export_markdown:
        os.makedirs(os.path.dirname(os.path.abspath(args.export_markdown)), exist_ok=True)
        with open(args.export_markdown, "w", encoding="utf-8") as handle:
            handle.write("# Benchmark Results\n\n")
            handle.write(
                "| Command | Mean ± σ [ms] | Median [ms] | p95 [ms] | Min [ms] | Max [ms] | Relative |\n"
            )
            handle.write("|:---|---:|---:|---:|---:|---:|---:|\n")
            for result in results:
                relative = "—"
                if fastest is not None:
                    relative = f"{result['mean'] / fastest['mean']:.2f}x"
                handle.write(
                    f"| `{result['command']}` | {result['mean']*1000:.2f} ± {result['stddev']*1000:.2f} | "
                    f"{result['median']*1000:.2f} | {result['p95']*1000:.2f} | "
                    f"{result['min']*1000:.2f} | {result['max']*1000:.2f} | {relative} |\n"
                )


if __name__ == "__main__":
    main()
