#!/usr/bin/env python3
"""Collect resource metrics for pkg benchmark commands using GNU time -v."""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import tempfile
import time

FIELDS = {
    "User time (seconds)": "user_s",
    "System time (seconds)": "system_s",
    "Maximum resident set size (kbytes)": "max_rss_kib",
    "File system inputs": "fs_inputs",
    "File system outputs": "fs_outputs",
    "Major (requiring I/O) page faults": "major_faults",
    "Minor (reclaiming a frame) page faults": "minor_faults",
    "Voluntary context switches": "voluntary_ctx",
    "Involuntary context switches": "involuntary_ctx",
}


def run_prepare(command: str) -> None:
    result = subprocess.run(
        ["/bin/bash", "-lc", command],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"prepare command failed with exit code {result.returncode}: {command}")


def parse_time_file(path: str) -> dict[str, float]:
    metrics: dict[str, float] = {}
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            if ":" not in line:
                continue
            key, value = line.strip().split(":", 1)
            if key not in FIELDS:
                continue
            try:
                metrics[FIELDS[key]] = float(value.strip())
            except ValueError:
                continue
    missing = [name for name in FIELDS.values() if name not in metrics]
    if missing:
        raise RuntimeError(f"GNU time output missing metrics: {', '.join(missing)}")
    return metrics


def measure(command: str) -> dict[str, float]:
    env = os.environ.copy()
    env["LC_ALL"] = "C"
    with tempfile.NamedTemporaryFile(prefix="pkg-time-", delete=False) as tmp:
        path = tmp.name
    try:
        started = time.perf_counter()
        result = subprocess.run(
            ["/usr/bin/time", "-v", "-o", path, "/bin/bash", "-lc", command],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env=env,
            check=False,
        )
        wall = time.perf_counter() - started
        if result.returncode != 0:
            raise RuntimeError(
                f"benchmark command failed with exit code {result.returncode}: {command}"
            )
        metrics = parse_time_file(path)
        metrics["wall_s"] = wall
        return metrics
    finally:
        try:
            os.unlink(path)
        except FileNotFoundError:
            pass


def mean(samples: list[dict[str, float]], field: str) -> float:
    return statistics.fmean(sample[field] for sample in samples)


def main() -> None:
    parser = argparse.ArgumentParser(description="pkg resource benchmark runner")
    parser.add_argument("--name", action="append", required=True)
    parser.add_argument("--command", "-c", action="append", required=True)
    parser.add_argument("--prepare", "-p", action="append", required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--export-json", required=True)
    parser.add_argument("--export-markdown", required=True)
    args = parser.parse_args()

    if not (len(args.name) == len(args.command) == len(args.prepare)):
        parser.error("provide one --name and --prepare for each --command")
    if args.runs < 1:
        parser.error("--runs must be >= 1")
    if not os.path.exists("/usr/bin/time"):
        raise SystemExit("GNU /usr/bin/time is required for resource metrics")

    results = []
    for name, command, prepare in zip(args.name, args.command, args.prepare):
        print(f"Resource benchmark: {name} ({args.runs} runs)")
        samples = []
        for _ in range(args.runs):
            run_prepare(prepare)
            samples.append(measure(command))
        summary = {
            "command": name,
            "raw_command": command,
            "runs": args.runs,
            "wall_s": mean(samples, "wall_s"),
            "user_s": mean(samples, "user_s"),
            "system_s": mean(samples, "system_s"),
            "max_rss_kib": mean(samples, "max_rss_kib"),
            "fs_inputs": mean(samples, "fs_inputs"),
            "fs_outputs": mean(samples, "fs_outputs"),
            "major_faults": mean(samples, "major_faults"),
            "minor_faults": mean(samples, "minor_faults"),
            "voluntary_ctx": mean(samples, "voluntary_ctx"),
            "involuntary_ctx": mean(samples, "involuntary_ctx"),
            "samples": samples,
        }
        results.append(summary)

    os.makedirs(os.path.dirname(os.path.abspath(args.export_json)), exist_ok=True)
    with open(args.export_json, "w", encoding="utf-8") as handle:
        json.dump({"results": results}, handle, indent=2)

    os.makedirs(os.path.dirname(os.path.abspath(args.export_markdown)), exist_ok=True)
    with open(args.export_markdown, "w", encoding="utf-8") as handle:
        handle.write("# Resource Benchmark Results\n\n")
        handle.write(
            "| Command | Wall [ms] | User [ms] | System [ms] | Peak RSS [KiB] | FS in ops | FS out ops | Major faults |\n"
        )
        handle.write("|:---|---:|---:|---:|---:|---:|---:|---:|\n")
        for result in results:
            handle.write(
                f"| `{result['command']}` | {result['wall_s']*1000:.2f} | "
                f"{result['user_s']*1000:.2f} | {result['system_s']*1000:.2f} | "
                f"{result['max_rss_kib']:.0f} | {result['fs_inputs']:.1f} | "
                f"{result['fs_outputs']:.1f} | {result['major_faults']:.1f} |\n"
            )


if __name__ == "__main__":
    main()
