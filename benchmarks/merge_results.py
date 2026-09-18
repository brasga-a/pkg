#!/usr/bin/env python3
"""Merge one-result benchmark JSON files into a publication-friendly report."""

from __future__ import annotations

import argparse
import json
import math
import os


def percentile(values: list[float], p: float) -> float:
    ordered = sorted(values)
    if not ordered:
        return 0.0
    index = max(0, math.ceil(p * len(ordered)) - 1)
    return ordered[index]


def main() -> None:
    parser = argparse.ArgumentParser(description="Merge pkg benchmark result files")
    parser.add_argument("--input", action="append", required=True)
    parser.add_argument("--name", action="append", default=[])
    parser.add_argument("--output-json", required=True)
    parser.add_argument("--output-markdown", required=True)
    parser.add_argument("--comparable", action="store_true")
    parser.add_argument("--note")
    args = parser.parse_args()

    if args.name and len(args.name) != len(args.input):
        parser.error("when --name is used, provide exactly one name for each --input")

    results = []
    for index, path in enumerate(args.input):
        with open(path, "r", encoding="utf-8") as handle:
            payload = json.load(handle)
        entries = payload.get("results", [])
        if len(entries) != 1:
            raise SystemExit(f"expected exactly one result in {path}, got {len(entries)}")
        result = dict(entries[0])
        if args.name:
            result["raw_command"] = result.get("raw_command", result.get("command"))
            result["command"] = args.name[index]
        times = [float(value) for value in result.get("times", [])]
        if times:
            result["p95"] = percentile(times, 0.95)
        else:
            result.setdefault("p95", result.get("max", result.get("mean", 0.0)))
        results.append(result)

    fastest = min(results, key=lambda item: item["mean"]) if args.comparable else None
    output = {"results": results}
    if args.note:
        output["note"] = args.note

    os.makedirs(os.path.dirname(os.path.abspath(args.output_json)), exist_ok=True)
    with open(args.output_json, "w", encoding="utf-8") as handle:
        json.dump(output, handle, indent=2)

    os.makedirs(os.path.dirname(os.path.abspath(args.output_markdown)), exist_ok=True)
    with open(args.output_markdown, "w", encoding="utf-8") as handle:
        handle.write("# Benchmark Results\n\n")
        if args.note:
            handle.write(f"> {args.note}\n\n")
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
                f"{result.get('median', result['mean'])*1000:.2f} | {result['p95']*1000:.2f} | "
                f"{result['min']*1000:.2f} | {result['max']*1000:.2f} | {relative} |\n"
            )


if __name__ == "__main__":
    main()
