#!/usr/bin/env python3
"""Compare system information commands with GNU uname."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess  # nosec B404 -- Validated executables, never a shell.
import time


def executable(path):
    result = path.resolve(strict=True)
    if not result.is_file() or not os.access(result, os.X_OK):
        raise ValueError(f"Expected an executable file: {result}")
    return result


def measure(options):
    binaries = {"gutil": executable(options.gutil), "gnu": executable(options.gnu)}
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    generator = random.Random(42)  # nosec B311 -- Reproducible invocation order only.
    results = {}
    for name, candidate, reference in [
        ("kernel", ["uname"], []),
        ("labeled", ["uname", "-A"], ["-A"]),
        ("machine", ["arch"], ["-m"]),
    ]:
        commands = {
            "gutil": [str(binaries["gutil"]), "--no-config", "--progress=never", *candidate],
            "gnu": [str(binaries["gnu"]), *reference],
        }
        samples = {label: [] for label in commands}
        expected = None
        for round_index in range(options.rounds + options.warmup):
            labels = list(commands)
            generator.shuffle(labels)
            for label in labels:
                argv = commands[label]
                start = time.perf_counter_ns()
                # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
                result = subprocess.run(argv,  # nosec B603 -- Validated executable, shell=False.
                    stdin=subprocess.DEVNULL, capture_output=True, env=environment,
                    check=False, shell=False,
                )
                elapsed = (time.perf_counter_ns() - start) / 1_000_000
                if result.returncode or result.stderr or not result.stdout.endswith(b"\n"):
                    raise RuntimeError(f"Unexpected {name} result: {result!r}")
                if expected is None:
                    expected = result.stdout
                if expected != result.stdout:
                    raise RuntimeError(f"Mismatched {name} output")
                if round_index >= options.warmup:
                    samples[label].append(elapsed)
        results[name] = {
            "candidate_args": candidate,
            "reference_args": reference,
            "median_ms": {label: statistics.median(values) for label, values in samples.items()},
            "samples_ms": samples,
        }
    return {
        "platform": platform.platform(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rounds": options.rounds,
        "warmup": options.warmup,
        "seed": 42,
        "binaries": {
            label: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for label, path in binaries.items()
        },
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/uname"))
    parser.add_argument("--rounds", type=int, default=100)
    parser.add_argument("--warmup", type=int, default=10)
    options = parser.parse_args()
    if options.rounds < 1 or options.warmup < 0:
        parser.error("rounds must be positive and warmup must be nonnegative")
    print(json.dumps(measure(options), indent=2))


if __name__ == "__main__":
    main()
