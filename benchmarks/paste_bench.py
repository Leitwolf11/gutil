#!/usr/bin/env python3
"""Compare gutil and GNU paste on regular files with verified output."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess  # nosec B404 -- Direct invocation of validated executables.
import tempfile
import time


def executable(path):
    result = path.resolve(strict=True)
    if not result.is_file() or not os.access(result, os.X_OK):
        raise ValueError(f"Expected an executable file: {result}")
    return result


def invoke(argv, root, environment, stdout):
    start = time.perf_counter_ns()
    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
    result = subprocess.run(argv,  # nosec B603 -- Validated executable and argv, shell=False.
        cwd=root, env=environment, stdin=subprocess.DEVNULL, stdout=stdout,
        stderr=subprocess.PIPE, check=True, shell=False,
    )
    elapsed = (time.perf_counter_ns() - start) / 1_000_000
    if result.stderr:
        raise RuntimeError(f"Unexpected diagnostics: {result.stderr!r}")
    return elapsed, result.stdout


def measure(args):
    binaries = {"gutil": executable(args.gutil), "gnu": executable(args.gnu)}
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    environment.pop("POSIXLY_CORRECT", None)
    rng = random.Random(42)  # nosec B311 -- Deterministic sample ordering.
    results = {}
    with tempfile.TemporaryDirectory(prefix="gutil-paste-bench-") as directory:
        root = Path(directory)
        rows = 131_072
        patterns = (b"a" * 95 + b"\n", b"b" * 95 + b"\n", b"c" * 95 + b"\n")
        paths = ["a", "b", "c"]
        for path, pattern in zip(paths, patterns, strict=True):
            (root / path).write_bytes(pattern * rows)
        for name, flags in [("parallel", []), ("serial", ["-s"]), ("cyclic", ["-d:;"])]:
            commands = {
                "gutil": [str(binaries["gutil"]), "--no-config", "--progress=never", "--jobs=1",
                          "paste", *flags, *paths],
                "gnu": [str(binaries["gnu"]), *flags, *paths],
            }
            outputs = {
                label: hashlib.sha256(invoke(argv, root, environment, subprocess.PIPE)[1]).hexdigest()
                for label, argv in commands.items()
            }
            if outputs["gnu"] != outputs["gutil"]:
                raise RuntimeError(f"Mismatched output for {name}: {outputs}")
            samples = {label: [] for label in commands}
            for index in range(args.rounds + args.warmup):
                order = list(commands)
                rng.shuffle(order)
                for label in order:
                    elapsed, _ = invoke(commands[label], root, environment, subprocess.DEVNULL)
                    if index >= args.warmup:
                        samples[label].append(elapsed)
            results[name] = {
                "flags": flags, "output_sha256": outputs["gnu"], "samples_ms": samples,
                "median_ms": {label: statistics.median(values) for label, values in samples.items()},
            }
    return {
        "platform": platform.platform(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rows_per_file": rows, "bytes_per_row": len(patterns[0]),
        "rounds": args.rounds, "warmup": args.warmup, "seed": 42,
        "binaries": {
            label: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for label, path in binaries.items()
        },
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/paste"))
    parser.add_argument("--rounds", type=int, default=16)
    parser.add_argument("--warmup", type=int, default=3)
    args = parser.parse_args()
    if args.rounds < 2 or args.warmup < 0:
        parser.error("rounds must be at least two and warmup must be nonnegative")
    print(json.dumps(measure(args), indent=2))


if __name__ == "__main__":
    main()
