#!/usr/bin/env python3
"""Benchmark CLI command paths added in 0.1.0-alpha.2.

This suite focuses on command-level latency, not cross-tool speed claims. It uses
isolated pkg data directories and a deterministic local Debian-style repository
served from 127.0.0.1 so `update`, `search`, and `repo` benchmarks do not depend
on external network latency.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import http.server
import json
import math
import os
import platform
import shutil
import socketserver
import statistics
import subprocess
import tempfile
import threading
import time
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parents[1]
BENCH_DIR = ROOT_DIR / "benchmarks"
DEFAULT_RESULTS = BENCH_DIR / "results"
DEFAULT_DEB = ROOT_DIR / "examples" / "hello_world" / "hello-world_1.0.0_amd64.deb"
PKG_BIN = ROOT_DIR / "target" / "release" / "pkg"


def run_checked(argv: list[str], *, cwd: Path | None = None) -> None:
    result = subprocess.run(
        argv,
        cwd=cwd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(argv)}")


def output_checked(argv: list[str]) -> str:
    result = subprocess.run(argv, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(argv)}\n{result.stderr}"
        )
    return result.stdout


def percentile(values: list[float], p: float) -> float:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(p * len(ordered)) - 1)]


def summarize(times: list[float]) -> dict[str, object]:
    return {
        "runs": len(times),
        "mean": statistics.fmean(times),
        "stddev": statistics.stdev(times) if len(times) > 1 else 0.0,
        "median": statistics.median(times),
        "p95": percentile(times, 0.95),
        "min": min(times),
        "max": max(times),
        "times": times,
    }


def clone_state(src: Path, dst: Path) -> None:
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(src, dst)


def benchmark(
    name: str,
    command: list[str],
    baseline: Path,
    run_dir: Path,
    runs: int,
    warmup: int,
) -> dict[str, object]:
    def one() -> float:
        clone_state(baseline, run_dir)
        started = time.perf_counter_ns()
        run_checked(command)
        return (time.perf_counter_ns() - started) / 1_000_000_000

    for _ in range(warmup):
        one()

    times = [one() for _ in range(runs)]
    result = summarize(times)
    result["command"] = name
    result["argv"] = command
    return result


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, _format: str, *_args: object) -> None:
        pass


class LocalRepoServer:
    def __init__(self, root: Path):
        handler = lambda *args, **kwargs: QuietHandler(*args, directory=str(root), **kwargs)
        self.httpd = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
        self.httpd.daemon_threads = True
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    @property
    def url(self) -> str:
        host, port = self.httpd.server_address
        return f"http://{host}:{port}"

    def start(self) -> None:
        self.thread.start()

    def close(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=2)


def build_local_repository(root: Path, count: int) -> str:
    packages_dir = root / "dists" / "bench" / "main" / "binary-amd64"
    packages_dir.mkdir(parents=True, exist_ok=True)
    (root / "dists" / "bench" / "InRelease").write_text(
        "Origin: pkg-benchmark\nSuite: bench\nCodename: bench\nArchitectures: amd64\nComponents: main\n",
        encoding="utf-8",
    )

    target_name = "bench-target"
    blocks: list[str] = []
    target_index = count // 2
    for i in range(count):
        name = target_name if i == target_index else f"bench-pkg-{i:06d}"
        version = f"1.0.{i}"
        fake_payload = f"{name}-{version}".encode()
        digest = hashlib.sha256(fake_payload).hexdigest()
        blocks.append(
            "\n".join(
                [
                    f"Package: {name}",
                    f"Version: {version}",
                    "Architecture: amd64",
                    f"Filename: pool/main/{name}_{version}_amd64.deb",
                    f"SHA256: {digest}",
                    f"Size: {1000 + i}",
                ]
            )
        )

    text = "\n\n".join(blocks) + "\n\n"
    with gzip.open(packages_dir / "Packages.gz", "wb", compresslevel=6) as handle:
        handle.write(text.encode())
    return target_name


def detect_package_name(deb: Path, probe_dir: Path) -> str:
    if shutil.which("dpkg-deb"):
        result = subprocess.run(
            ["dpkg-deb", "-f", str(deb), "Package"],
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()

    text = output_checked([str(PKG_BIN), "--data-dir", str(probe_dir), "info", str(deb)])
    for line in text.splitlines():
        if line.strip().startswith("Name:"):
            return line.split(":", 1)[1].strip()
    raise RuntimeError("unable to detect package name")


def write_reports(run_root: Path, results: list[dict[str, object]], metadata: dict[str, object]) -> None:
    payload = {"metadata": metadata, "results": results}
    (run_root / "results.json").write_text(json.dumps(payload, indent=2), encoding="utf-8")

    lines = [
        "# pkg CLI Command Benchmark",
        "",
        "> Commands below have different semantics. Compare each row primarily against the same row from another commit/build, not against each other.",
        "",
        "| Command | Mean ± σ [ms] | Median [ms] | p95 [ms] | Min [ms] | Max [ms] |",
        "|:---|---:|---:|---:|---:|---:|",
    ]
    for item in results:
        lines.append(
            "| `{}` | {:.3f} ± {:.3f} | {:.3f} | {:.3f} | {:.3f} | {:.3f} |".format(
                item["command"],
                float(item["mean"]) * 1000,
                float(item["stddev"]) * 1000,
                float(item["median"]) * 1000,
                float(item["p95"]) * 1000,
                float(item["min"]) * 1000,
                float(item["max"]) * 1000,
            )
        )
    (run_root / "results.md").write_text("\n".join(lines) + "\n", encoding="utf-8")

    env_lines = [f"{key}: {value}" for key, value in metadata.items()]
    (run_root / "environment.txt").write_text("\n".join(env_lines) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description="Benchmark pkg 0.1.0-alpha.2 CLI commands")
    parser.add_argument("package", nargs="?", default=str(DEFAULT_DEB))
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--catalog-size", type=int, default=1000)
    parser.add_argument("--profile", default="benchmark-cli")
    parser.add_argument("--output-dir", default=str(DEFAULT_RESULTS))
    args = parser.parse_args()

    if args.runs < 1 or args.warmup < 0 or args.catalog_size < 1:
        parser.error("runs >= 1, warmup >= 0 and catalog-size >= 1 are required")

    package = Path(args.package).expanduser().resolve()
    if not PKG_BIN.exists():
        print("Building release binary...")
        run_checked(["cargo", "build", "--release"], cwd=ROOT_DIR)

    if package == DEFAULT_DEB.resolve() and not package.exists():
        run_checked(["./build.sh"], cwd=DEFAULT_DEB.parent)
    if not package.is_file():
        raise SystemExit(f"package not found: {package}")

    temp_root = Path(tempfile.mkdtemp(prefix="pkg-alpha2-cli-bench-"))
    server: LocalRepoServer | None = None
    try:
        repo_root = temp_root / "repo"
        target_remote = build_local_repository(repo_root, args.catalog_size)
        server = LocalRepoServer(repo_root)
        server.start()

        empty = temp_root / "baseline-empty"
        installed = temp_root / "baseline-installed"
        repo_configured = temp_root / "baseline-repo-configured"
        catalog = temp_root / "baseline-catalog"
        run_state = temp_root / "run-state"
        probe = temp_root / "probe"

        run_checked([str(PKG_BIN), "--data-dir", str(empty), "--profile", args.profile, "list"])
        package_name = detect_package_name(package, probe)

        clone_state(empty, installed)
        run_checked(
            [str(PKG_BIN), "--data-dir", str(installed), "--profile", args.profile, "install", str(package)]
        )

        clone_state(empty, repo_configured)
        run_checked(
            [
                str(PKG_BIN), "--data-dir", str(repo_configured), "repo", "add",
                "bench-local", server.url, "bench", "main",
            ]
        )

        clone_state(repo_configured, catalog)
        run_checked([str(PKG_BIN), "--data-dir", str(catalog), "update"])
        search_output = output_checked([str(PKG_BIN), "--data-dir", str(catalog), "search", target_remote])
        if "Found package:" not in search_output:
            raise RuntimeError("catalog baseline sanity check failed: remote target not searchable")

        cases = [
            ("pkg list (empty)", [str(PKG_BIN), "--data-dir", str(run_state), "--profile", args.profile, "list"], empty),
            ("pkg list (installed)", [str(PKG_BIN), "--data-dir", str(run_state), "--profile", args.profile, "list"], installed),
            ("pkg remove", [str(PKG_BIN), "--data-dir", str(run_state), "--profile", args.profile, "remove", package_name], installed),
            ("pkg remove --dry-run", [str(PKG_BIN), "--data-dir", str(run_state), "--profile", args.profile, "remove", package_name, "--dry-run"], installed),
            ("pkg repo list (empty)", [str(PKG_BIN), "--data-dir", str(run_state), "repo", "list"], empty),
            ("pkg repo add", [str(PKG_BIN), "--data-dir", str(run_state), "repo", "add", "bench-extra", server.url, "bench", "main"], empty),
            ("pkg repo list (configured)", [str(PKG_BIN), "--data-dir", str(run_state), "repo", "list"], repo_configured),
            (f"pkg update ({args.catalog_size} packages)", [str(PKG_BIN), "--data-dir", str(run_state), "update"], repo_configured),
            ("pkg search (hit)", [str(PKG_BIN), "--data-dir", str(run_state), "search", target_remote], catalog),
            ("pkg search (miss)", [str(PKG_BIN), "--data-dir", str(run_state), "search", "benchmark-package-that-does-not-exist"], catalog),
            ("pkg install remote --dry-run", [str(PKG_BIN), "--data-dir", str(run_state), "install", target_remote, "--dry-run"], catalog),
        ]

        results: list[dict[str, object]] = []
        for index, (name, command, baseline) in enumerate(cases, 1):
            print(f"[{index:02d}/{len(cases):02d}] {name}")
            results.append(benchmark(name, command, baseline, run_state, args.runs, args.warmup))

        branch = subprocess.run(
            ["git", "-C", str(ROOT_DIR), "rev-parse", "--abbrev-ref", "HEAD"],
            text=True, capture_output=True, check=False,
        ).stdout.strip() or "unknown"
        commit = subprocess.run(
            ["git", "-C", str(ROOT_DIR), "rev-parse", "HEAD"],
            text=True, capture_output=True, check=False,
        ).stdout.strip() or "unknown"

        artifact_digest = hashlib.sha256(package.read_bytes()).hexdigest()
        timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
        run_root = Path(args.output_dir).expanduser().resolve() / f"{timestamp}-cli-alpha2"
        run_root.mkdir(parents=True, exist_ok=False)

        metadata = {
            "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "git_branch": branch,
            "git_commit": commit,
            "pkg_binary": str(PKG_BIN),
            "package": str(package),
            "package_sha256": artifact_digest,
            "package_name": package_name,
            "profile": args.profile,
            "runs": args.runs,
            "warmup": args.warmup,
            "catalog_size": args.catalog_size,
            "repository": server.url,
            "python": platform.python_version(),
            "kernel": platform.release(),
            "machine": platform.machine(),
            "processor": platform.processor() or "unknown",
        }
        write_reports(run_root, results, metadata)

        print(f"\nResults: {run_root}")
        print(f"  {run_root / 'results.md'}")
        print(f"  {run_root / 'results.json'}")
        print(f"  {run_root / 'environment.txt'}")
    finally:
        if server is not None:
            server.close()
        shutil.rmtree(temp_root, ignore_errors=True)


if __name__ == "__main__":
    main()
