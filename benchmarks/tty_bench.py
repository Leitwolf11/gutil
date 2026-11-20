#!/usr/bin/env python3
"""Compare gutil tty and GNU tty with redirected and pseudo-terminal stdin."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import pty
import random
import statistics
import subprocess  # nosec B404 -- Explicit benchmark executables, never a shell.
import time


def executable(path):
    path = path.resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"Expected an executable file: {path}")
    return path


def measure(options):
    binaries = {"gutil": executable(options.gutil), "gnu": executable(options.gnu)}
    commands = {
        "gutil": [str(binaries["gutil"]), "--no-config", "--progress=never", "tty"],
        "gnu": [str(binaries["gnu"])],
    }
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    environment.pop("POSIXLY_CORRECT", None)
    generator = random.Random(42)  # nosec B311 -- Reproducible invocation order only.
    results = {}
    master, slave = pty.openpty()
    try:
        for terminal in (False, True):
            for quiet in (False, True):
                mode = f'{"pty" if terminal else "redirected"}-{"quiet" if quiet else "print"}'
                samples = {label: [] for label in commands}
                name = os.fsencode(os.ttyname(slave)) if terminal else b"not a tty"
                expected = (int(not terminal), b"" if quiet else name + b"\n", b"")
                args = ["-s"] if quiet else []
                for round_index in range(options.rounds + options.warmup):
                    order = list(commands)
                    generator.shuffle(order)
                    for label in order:
                        started = time.perf_counter_ns()
                        # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
                        result = subprocess.run(  # nosec B603 -- Validated executable, shell=False.
                            commands[label] + args,
                            stdin=slave if terminal else subprocess.DEVNULL,
                            capture_output=True, env=environment, check=False, shell=False,
                        )
                        elapsed = (time.perf_counter_ns() - started) / 1_000_000
                        if (result.returncode, result.stdout, result.stderr) != expected:
                            raise RuntimeError(f"Unexpected result for {mode}: {result!r}")
                        if round_index >= options.warmup:
                            samples[label].append(elapsed)
                results[mode] = {
                    "samples_ms": samples,
                    "median_ms": {label: statistics.median(values) for label, values in samples.items()},
                }
    finally:
        os.close(slave)
        os.close(master)
    return {
        "platform": platform.platform(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rounds": options.rounds, "warmup": options.warmup, "seed": 42,
        "binaries": {
            label: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for label, path in binaries.items()
        },
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/tty"))
    parser.add_argument("--rounds", type=int, default=100)
    parser.add_argument("--warmup", type=int, default=10)
    options = parser.parse_args()
    if options.rounds < 1 or options.warmup < 0:
        parser.error("rounds must be positive and warmup must be nonnegative")
    print(json.dumps(measure(options), indent=2))


if __name__ == "__main__":
    main()
