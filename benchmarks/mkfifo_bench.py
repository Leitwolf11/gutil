#!/usr/bin/env python3
"""Compare release mkfifo with GNU on verified Unix FIFO workloads."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import stat
import statistics
import subprocess  # nosec B404 -- Explicit benchmark executables, never a shell.
import tempfile
import time


def executable(path):
    resolved = path.resolve(strict=True)
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        raise ValueError(f"Expected an executable file: {resolved}")
    return resolved


def run(command, root, environment):
    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
    result = subprocess.run(command,  # nosec B603 -- Validated executable, shell=False.
        cwd=root, env=environment, capture_output=True, shell=False, umask=0o027,
    )
    if result.returncode or result.stdout or result.stderr:
        raise RuntimeError(f"Unexpected FIFO creation result: {result!r}")


def measure(executables, root):
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    environment.pop("POSIXLY_CORRECT", None)
    generator = random.Random(918)  # nosec B311 -- Reproducible invocation order only.
    cases = [
        ("one-default", 1, [], 0o640),
        ("one-mode", 1, ["-m", "666"], 0o666),
        ("batch-default", 512, [], 0o640),
        ("batch-mode", 512, ["-m", "666"], 0o666),
    ]
    rounds, warmup = 24, 6
    results = {}
    for name, count, options, expected_mode in cases:
        operands = [f"fifo-{index}" for index in range(count)]
        samples = {label: [] for label in executables}
        for index in range(rounds + warmup):
            order = list(executables)
            generator.shuffle(order)
            for label in order:
                flags = ["--no-config", "--progress=never", "--jobs=1", "mkfifo"] if label != "gnu" else []
                started = time.perf_counter_ns()
                run([str(executables[label]), *flags, *options, *operands], root, environment)
                elapsed = (time.perf_counter_ns() - started) / 1_000_000
                for operand in operands:
                    path = root / operand
                    mode = path.lstat().st_mode
                    if not stat.S_ISFIFO(mode) or stat.S_IMODE(mode) != expected_mode:
                        raise RuntimeError(f"Unexpected FIFO mode from {label}: {mode:o}")
                    path.unlink()
                if index >= warmup:
                    samples[label].append(elapsed)
        results[name] = {
            "operands": count,
            "options": options,
            "mode_octal": f"{expected_mode:o}",
            "samples_ms": samples,
            "median_ms": {label: statistics.median(values) for label, values in samples.items()},
        }
    return {
        "system": platform.platform(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rounds": rounds,
        "warmup": warmup,
        "seed": 918,
        "umask": "027",
        "binaries": {label: hashlib.sha256(path.read_bytes()).hexdigest() for label, path in executables.items()},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/mkfifo"))
    args = parser.parse_args()
    binaries = {"gutil": executable(args.gutil), "gnu": executable(args.gnu)}
    if args.baseline is not None:
        binaries["baseline"] = executable(args.baseline)
    with tempfile.TemporaryDirectory(prefix="gutil-mkfifo-") as directory:
        print(json.dumps(measure(binaries, Path(directory)), indent=2))


if __name__ == "__main__":
    main()
