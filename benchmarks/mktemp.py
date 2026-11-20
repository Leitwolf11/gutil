#!/usr/bin/env python3
"""Compare release gutil and GNU mktemp while verifying every reserved object."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess  # nosec B404 -- Explicit benchmark binaries, never a shell.
import tempfile
import time


def executable(path):
    result = path.resolve(strict=True)
    if not result.is_file() or not os.access(result, os.X_OK):
        raise ValueError(f"Expected a regular executable: {result}")
    return result


def summarize(values):
    quartiles = statistics.quantiles(values, n=4, method="inclusive")
    return {"median": statistics.median(values), "q1": quartiles[0], "q3": quartiles[2]}


def invoke(arguments, root, environment):
    started = time.perf_counter_ns()
    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
    result = subprocess.run(arguments,  # nosec B603 -- Validated executable, shell=False.
        cwd=root, env=environment, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        check=False, shell=False, umask=0o022)
    elapsed = (time.perf_counter_ns() - started) / 1_000_000
    return elapsed, result


def verify(result, root, kind, default):
    if kind == "failure":
        if result.returncode != 1 or result.stdout or result.stderr or list(root.iterdir()):
            raise RuntimeError(f"Incorrect quiet-failure state: {result!r}")
        return
    if result.returncode or result.stderr or not result.stdout.endswith(b"\n"):
        raise RuntimeError(f"Benchmark failed: {result!r}")
    printed = Path(os.fsdecode(result.stdout[:-1]))
    path = root / printed
    prefix = "tmp." if default else "object."
    name = printed.name
    random_part = name.removeprefix(prefix)
    if not name.startswith(prefix) or len(random_part) != 10 or not random_part.isascii() or not random_part.isalnum():
        raise RuntimeError(f"Incorrect template replacement: {printed!r}")
    if path.parent != root or printed.is_absolute() != default:
        raise RuntimeError(f"Incorrect output location: {printed!r}")
    if kind == "name":
        if path.exists() or list(root.iterdir()):
            raise RuntimeError("Dry run changed the filesystem")
        return
    metadata = path.lstat()
    expected_mode = 0o700 if kind == "directory" else 0o600
    if metadata.st_mode & 0o7777 != expected_mode or len(list(root.iterdir())) != 1:
        raise RuntimeError(f"Incorrect mode or object count: {path!r}")
    if kind == "directory":
        if not path.is_dir() or list(path.iterdir()):
            raise RuntimeError(f"Incorrect directory: {path!r}")
    elif not path.is_file() or metadata.st_size:
        raise RuntimeError(f"Incorrect file: {path!r}")


def benchmark(options, root):
    binaries = {"gutil": executable(options.gutil), "gnu": executable(options.gnu)}
    if options.baseline is not None:
        binaries["baseline"] = executable(options.baseline)
    cases = [
        ("file", ["object.XXXXXXXXXX"], "file", False),
        ("directory", ["-d", "object.XXXXXXXXXX"], "directory", False),
        ("default-tmpdir", [], "file", True),
        ("dry-run", ["-u", "object.XXXXXXXXXX"], "name", False),
        ("quiet-missing-parent", ["-q", "missing/object.XXXXXXXXXX"], "failure", False),
    ]
    generator = random.Random(42)  # nosec B311 -- Reproducible invocation order only.
    work = root / "work"
    environment = dict(os.environ, LC_ALL="C", TZ="UTC", TMPDIR=str(work))
    results = {}
    for name, flags, kind, default in cases:
        commands = {
            "gutil": [str(binaries["gutil"]), "--no-config", "--progress=never", "--jobs=1", "mktemp", *flags],
            "gnu": [str(binaries["gnu"]), *flags],
        }
        if "baseline" in binaries:
            commands["baseline"] = [str(binaries["baseline"]), *commands["gutil"][1:]]
        samples = {label: [] for label in commands}
        for round_index in range(options.warmup + options.rounds):
            order = list(commands)
            generator.shuffle(order)
            for label in order:
                if work.exists():
                    shutil.rmtree(work)
                work.mkdir()
                elapsed, result = invoke(commands[label], work, environment)
                verify(result, work, kind, default)
                if round_index >= options.warmup:
                    samples[label].append(elapsed)
        reductions = [100 * (gnu - candidate) / gnu for candidate, gnu in zip(samples["gutil"], samples["gnu"], strict=True)]
        results[name] = {
            "arguments": flags, "kind": kind,
            "milliseconds": {label: summarize(values) for label, values in samples.items()},
            "paired_reduction_percent": summarize(reductions), "samples_ms": samples,
        }
        if "baseline" in samples:
            baseline_reductions = [
                100 * (baseline - candidate) / baseline
                for candidate, baseline in zip(samples["gutil"], samples["baseline"], strict=True)
            ]
            results[name]["paired_baseline_reduction_percent"] = summarize(baseline_reductions)
    return {
        "measured_at_utc": datetime.datetime.now(datetime.UTC).isoformat(),
        "machine": platform.platform(), "cpu": platform.processor(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "fixture_parent": str(root.parent),
        "rounds": options.rounds, "warmup": options.warmup, "seed": 42, "umask": "022",
        "binary_size_method": "Filesystem logical length from stat().st_size",
        "binaries": {label: {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "size_bytes": binary.stat().st_size} for label, binary in binaries.items()},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--baseline", type=Path, help="Optional second gutil release binary")
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/mktemp"))
    parser.add_argument("--rounds", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=4)
    options = parser.parse_args()
    if os.name != "posix":
        parser.error("This harness compares native POSIX modes and GNU mktemp")
    if options.rounds < 2 or options.warmup < 0:
        parser.error("--rounds must be at least two and --warmup nonnegative")
    previous = os.umask(0o022)
    try:
        with tempfile.TemporaryDirectory(prefix="gutil-mktemp-bench-") as temporary:
            print(json.dumps(benchmark(options, Path(temporary)), indent=2))
    finally:
        os.umask(previous)


if __name__ == "__main__":
    main()
