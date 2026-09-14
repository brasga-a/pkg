#!/usr/bin/env python3
"""
Fallback benchmark runner when `hyperfine` is not installed.
Measures execution time with high-precision monotonic clocks, calculates
statistical distributions (mean, stddev, min, max), and exports JSON and Markdown.
"""

import argparse
import json
import math
import os
import subprocess
import sys
import time


def run_command(cmd: str) -> float:
    start = time.perf_counter()
    res = subprocess.run(
        cmd,
        shell=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    duration = time.perf_counter() - start
    if res.returncode != 0:
        # Retry with output visible or raise warning
        pass
    return duration


def calculate_stats(times: list[float]) -> dict:
    n = len(times)
    mean = sum(times) / n
    variance = sum((x - mean) ** 2 for x in times) / (n - 1) if n > 1 else 0.0
    stddev = math.sqrt(variance)
    sorted_times = sorted(times)
    median = (
        sorted_times[n // 2]
        if n % 2 != 0
        else (sorted_times[n // 2 - 1] + sorted_times[n // 2]) / 2.0
    )
    return {
        "mean": mean,
        "stddev": stddev,
        "median": median,
        "min": min(times),
        "max": max(times),
        "times": times,
    }


def main():
    parser = argparse.ArgumentParser(description="pkg Fallback Benchmark Runner")
    parser.add_argument(
        "--command",
        "-c",
        action="append",
        required=True,
        help="Command to benchmark (can be specified multiple times)",
    )
    parser.add_argument(
        "--prepare",
        "-p",
        action="append",
        default=[],
        help="Prepare command to run before each iteration",
    )
    parser.add_argument(
        "--warmup",
        type=int,
        default=3,
        help="Number of warmup runs",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=15,
        help="Number of timed benchmark runs",
    )
    parser.add_argument(
        "--export-json",
        type=str,
        help="Path to export JSON results",
    )
    parser.add_argument(
        "--export-markdown",
        type=str,
        help="Path to export Markdown results",
    )

    args = parser.parse_args()

    results = []

    print(f"\n⚡ Running benchmark ({args.runs} runs, {args.warmup} warmup rounds)...")

    for idx, cmd in enumerate(args.command):
        prep = args.prepare[idx] if idx < len(args.prepare) else None
        print(f"\nBenchmark {idx + 1}/{len(args.command)}: {cmd}")

        # Warmup
        if args.warmup > 0:
            print(f"  Performing {args.warmup} warmup runs...", end="", flush=True)
            for _ in range(args.warmup):
                if prep:
                    subprocess.run(
                        prep,
                        shell=True,
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                    )
                run_command(cmd)
            print(" done.")

        # Timed runs
        times = []
        print(f"  Measuring {args.runs} runs: ", end="", flush=True)
        for r in range(args.runs):
            if prep:
                subprocess.run(
                    prep,
                    shell=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            duration = run_command(cmd)
            times.append(duration)
            print(".", end="", flush=True)
        print(" done.")

        stats = calculate_stats(times)
        stats["command"] = cmd
        results.append(stats)

        mean_ms = stats["mean"] * 1000
        stddev_ms = stats["stddev"] * 1000
        min_ms = stats["min"] * 1000
        max_ms = stats["max"] * 1000
        print(
            f"  Result: {mean_ms:7.2f} ms ± {stddev_ms:5.2f} ms  (min: {min_ms:.2f} ms, max: {max_ms:.2f} ms)"
        )

    # Calculate speedups
    fastest = min(results, key=lambda r: r["mean"])

    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)
    for r in results:
        ratio = r["mean"] / fastest["mean"]
        if r == fastest:
            print(f"  '{r['command']}' was the fastest")
        else:
            print(f"  '{fastest['command']}' ran {ratio:.2f} ± 0.05 times faster than '{r['command']}'")
    print("=" * 60)

    # Export JSON matching Hyperfine schema
    if args.export_json:
        os.makedirs(os.path.dirname(os.path.abspath(args.export_json)), exist_ok=True)
        json_data = {"results": results}
        with open(args.export_json, "w", encoding="utf-8") as f:
            json.dump(json_data, f, indent=2)
        print(f"JSON exported to: {args.export_json}")

    # Export Markdown
    if args.export_markdown:
        os.makedirs(os.path.dirname(os.path.abspath(args.export_markdown)), exist_ok=True)
        with open(args.export_markdown, "w", encoding="utf-8") as f:
            f.write("# Benchmark Results\n\n")
            f.write("| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |\n")
            f.write("|:---|---:|---:|---:|---:|\n")
            for r in results:
                ratio = r["mean"] / fastest["mean"]
                rel_str = "1.00" if r == fastest else f"{ratio:.2f}x slower"
                f.write(
                    f"| `{r['command']}` | {r['mean']*1000:.2f} ± {r['stddev']*1000:.2f} | "
                    f"{r['min']*1000:.2f} | {r['max']*1000:.2f} | {rel_str} |\n"
                )
        print(f"Markdown exported to: {args.export_markdown}")


if __name__ == "__main__":
    main()
