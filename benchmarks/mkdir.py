#!/usr/bin/env python3
"""Compare release gutil mkdir and GNU mkdir on identical directory workloads."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess  # nosec B404 -- Explicit benchmark executables, never a shell.
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
    result = subprocess.run(arguments,  # nosec B603 -- Validated benchmark executable, shell=False.
        cwd=root, env=environment, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        check=False, shell=False, umask=0o022)
    elapsed = (time.perf_counter_ns() - started) / 1_000_000
    if result.returncode or result.stderr:
        raise RuntimeError(f"Benchmark failed: {arguments!r}: {result.returncode}: {result.stderr!r}")
    return elapsed


def verify(root, expected, permissions):
    actual = {}
    for directory, children, files in os.walk(root):
        if files:
            raise RuntimeError(f"Unexpected files: {files!r}")
        for child in children:
            path = Path(directory) / child
            actual[path.relative_to(root).as_posix()] = path.stat().st_mode & 0o7777
    if actual != dict.fromkeys(expected, permissions):
        raise RuntimeError(f"Incorrect directory tree: {actual!r}")


def benchmark(options, root):
    binaries = {"gutil": executable(options.gutil), "gnu": executable(options.gnu)}
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    siblings = [f"d{index:04}" for index in range(options.count)]
    nested = [f"p{index:04}/a/b/leaf" for index in range(options.count // 4)]
    ancestors = sorted({parent.as_posix() for name in nested for parent in [Path(name), *Path(name).parents] if parent != Path(".")})
    cases = [
        ("siblings", [], siblings, siblings, 0o755, False),
        ("siblings-mode-750", ["-m750"], siblings, siblings, 0o750, False),
        ("parents-depth-four", ["-p"], nested, ancestors, 0o755, False),
        ("existing-parents", ["-p"], siblings, siblings, 0o755, True),
    ]
    results = {}
    generator = random.Random(42)  # nosec B311 -- Reproducible invocation order only.
    for name, flags, operands, expected, permissions, existing in cases:
        commands = {
            "gutil": [str(binaries["gutil"]), "--no-config", "--progress=never", "--jobs=1", "mkdir", *flags, *operands],
            "gnu": [str(binaries["gnu"]), *flags, *operands],
        }
        samples = {label: [] for label in commands}
        work = root / "work"
        for round_index in range(options.warmup + options.rounds):
            order = list(commands)
            generator.shuffle(order)
            for label in order:
                if work.exists():
                    shutil.rmtree(work)
                work.mkdir()
                if existing:
                    for operand in operands:
                        (work / operand).mkdir(mode=0o755)
                elapsed = invoke(commands[label], work, environment)
                verify(work, expected, permissions)
                if round_index >= options.warmup:
                    samples[label].append(elapsed)
        reductions = [100 * (gnu - actual) / gnu for actual, gnu in zip(samples["gutil"], samples["gnu"], strict=True)]
        results[name] = {
            "directories": len(expected), "operands": len(operands),
            "milliseconds": {label: summarize(values) for label, values in samples.items()},
            "paired_reduction_percent": summarize(reductions), "samples_ms": samples,
        }
    return {
        "machine": platform.platform(), "cpu": platform.processor(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rounds": options.rounds, "warmup": options.warmup, "seed": 42, "umask": "022",
        "binaries": {label: {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()} for label, binary in binaries.items()},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/mkdir"))
    parser.add_argument("--count", type=int, default=512)
    parser.add_argument("--rounds", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=4)
    options = parser.parse_args()
    if os.name != "posix":
        parser.error("This harness compares native POSIX permission semantics with GNU mkdir")
    if options.count < 4 or options.count % 4 or options.rounds < 2 or options.warmup < 0:
        parser.error("--count must be positive and divisible by four, --rounds at least two, and --warmup nonnegative")
    previous = os.umask(0o022)
    try:
        with tempfile.TemporaryDirectory(prefix="gutil-mkdir-") as temporary:
            print(json.dumps(benchmark(options, Path(temporary)), indent=2))
    finally:
        os.umask(previous)


if __name__ == "__main__":
    main()
