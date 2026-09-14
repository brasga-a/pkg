#!/usr/bin/env python3
"""Render pkg benchmark JSON as an ASCII chart and optional PNG."""

from __future__ import annotations

import argparse
import json
import os


def load_results(path: str) -> list[dict]:
    with open(path, "r", encoding="utf-8") as handle:
        payload = json.load(handle)
    results = payload.get("results", [])
    if not results:
        raise SystemExit(f"no benchmark results found in {path}")
    return results


def ascii_chart(results: list[dict], title: str) -> None:
    labels = [str(result.get("command", "command")) for result in results]
    means_ms = [float(result["mean"]) * 1000 for result in results]
    std_ms = [float(result.get("stddev", 0.0)) * 1000 for result in results]
    maximum = max(means_ms) or 1.0
    width = 42
    label_width = min(max(len(label) for label in labels), 48)

    print("\n" + "=" * 72)
    print(f"Benchmark Visualization — {title} (lower is better)")
    print("=" * 72)
    for label, mean, stddev in zip(labels, means_ms, std_ms):
        bar = "█" * max(1, round((mean / maximum) * width))
        shown = label if len(label) <= label_width else label[: label_width - 1] + "…"
        print(f"{shown:<{label_width}}  {bar:<{width}}  {mean:9.2f} ms (±{stddev:.2f})")
    print("=" * 72)


def png_chart(results: list[dict], output: str, title: str) -> None:
    try:
        import matplotlib.pyplot as plt
    except ImportError:
        print("matplotlib not installed; skipping PNG generation")
        return

    labels = [str(result.get("command", "command")) for result in results]
    means_ms = [float(result["mean"]) * 1000 for result in results]
    std_ms = [float(result.get("stddev", 0.0)) * 1000 for result in results]

    fig, ax = plt.subplots(figsize=(10, max(4, len(labels) * 0.8)))
    positions = list(range(len(labels)))
    ax.barh(positions, means_ms, xerr=std_ms, capsize=4)
    ax.set_yticks(positions, labels=labels)
    ax.invert_yaxis()
    ax.set_xlabel("Mean wall-clock latency [ms]")
    ax.set_title(title)
    ax.grid(axis="x", alpha=0.25)
    fig.tight_layout()

    os.makedirs(os.path.dirname(os.path.abspath(output)), exist_ok=True)
    fig.savefig(output, dpi=180, bbox_inches="tight")
    plt.close(fig)
    print(f"PNG exported to: {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Plot pkg benchmark results")
    parser.add_argument("--input", default="benchmarks/results/results.json")
    parser.add_argument("--output", default="benchmarks/results/benchmark_chart.png")
    parser.add_argument("--title", default="pkg Benchmark")
    args = parser.parse_args()

    results = load_results(args.input)
    ascii_chart(results, args.title)
    png_chart(results, args.output, args.title)


if __name__ == "__main__":
    main()
