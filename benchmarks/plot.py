#!/usr/bin/env python3
"""
Plots benchmark results from results.json.
Outputs an ASCII bar chart in the console, and optionally a PNG chart using matplotlib.
"""

import argparse
import json
import os
import sys


def print_ascii_chart(results: list[dict]):
    print("\n" + "=" * 65)
    print(" 📊 Benchmark Visualization (Lower is better)")
    print("=" * 65)

    if not results:
        print("No results to display.")
        return

    max_mean = max(r["mean"] for r in results)
    bar_width = 35

    for r in results:
        cmd = r.get("command", "")
        # Truncate command name if too long
        if len(cmd) > 28:
            cmd_label = cmd[:25] + "..."
        else:
            cmd_label = cmd.ljust(28)

        mean_ms = r["mean"] * 1000
        stddev_ms = r.get("stddev", 0) * 1000

        bar_len = int((r["mean"] / max_mean) * bar_width) if max_mean > 0 else 0
        bar = "█" * max(1, bar_len)

        print(f"{cmd_label} | {bar} {mean_ms:6.2f} ms (±{stddev_ms:4.2f})")

    print("=" * 65 + "\n")


def generate_png_chart(results: list[dict], output_path: str):
    try:
        import matplotlib.pyplot as plt
    except ImportError:
        print("💡 Tip: Install matplotlib (`pip install matplotlib`) to generate high-res PNG charts.")
        return

    commands = [r.get("command", f"Cmd {i}") for i, r in enumerate(results)]
    means = [r["mean"] * 1000 for r in results]
    stddevs = [r.get("stddev", 0) * 1000 for r in results]

    # Clean command names for labels
    labels = []
    for c in commands:
        if "pkg" in c and "install" in c:
            labels.append("pkg install\n(Rust Rootless)")
        elif "dpkg" in c and "-i" in c:
            labels.append("dpkg -i\n(Host Native)")
        elif "pkg" in c and "info" in c:
            labels.append("pkg info")
        elif "pkg" in c and "remove" in c:
            labels.append("pkg remove")
        else:
            labels.append(c if len(c) <= 25 else c[:22] + "...")

    # Set dark theme style
    plt.style.use("dark_background")
    fig, ax = plt.subplots(figsize=(9, 5), dpi=150)

    colors = ["#38bdf8" if "pkg" in c else "#f87171" for c in commands]

    bars = ax.barh(labels, means, xerr=stddevs, capsize=5, color=colors, height=0.55)

    ax.set_xlabel("Time (milliseconds) — Lower is better", fontsize=11, labelpad=10)
    ax.set_title("Package Installation Benchmark (pkg vs Native)", fontsize=13, weight="bold", pad=15)
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)
    ax.grid(axis="x", linestyle="--", alpha=0.3)

    # Annotate bar values
    for bar, mean, std in zip(bars, means, stddevs):
        ax.text(
            bar.get_width() + (max(means) * 0.02),
            bar.get_y() + bar.get_height() / 2,
            f"{mean:.2f} ms",
            va="center",
            ha="left",
            fontsize=10,
            fontweight="bold",
            color="#f1f5f9",
        )

    plt.tight_layout()
    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
    plt.savefig(output_path)
    print(f"🖼️  Chart saved successfully to: {output_path}")


def main():
    parser = argparse.ArgumentParser(description="Plot benchmark results")
    parser.add_argument(
        "--input",
        "-i",
        default="benchmarks/results/results.json",
        help="Path to results.json (default: benchmarks/results/results.json)",
    )
    parser.add_argument(
        "--output",
        "-o",
        default="benchmarks/results/benchmark_chart.png",
        help="Path to save output chart (default: benchmarks/results/benchmark_chart.png)",
    )

    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: Input file '{args.input}' not found. Run `./benchmarks/run.sh` first.")
        sys.exit(1)

    with open(args.input, "r", encoding="utf-8") as f:
        data = json.load(f)

    results = data.get("results", [])
    print_ascii_chart(results)
    generate_png_chart(results, args.output)


if __name__ == "__main__":
    main()
