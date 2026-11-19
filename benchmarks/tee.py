#!/usr/bin/env python3
"""Compare release gutil tee with GNU tee using verified interleaved runs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess  # nosec B404 -- Explicit benchmark executables, never a shell.
import tempfile
import time


def executable(path):
    resolved = path.resolve(strict=True)
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        raise ValueError(f"Expected a regular executable file: {resolved}")
    return resolved


def invoke(arguments, directory, source, environment, piped, output=subprocess.DEVNULL):
    with source.open("rb") as stream:
        # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
        child = subprocess.Popen(arguments,  # nosec B603 -- Validated benchmark executable, shell=False.
            cwd=directory, env=environment, stdin=subprocess.PIPE if piped else stream,
            stdout=output, stderr=subprocess.PIPE, shell=False)
        if piped:
            try:
                while block := stream.read(128 * 1024):
                    child.stdin.write(block)
                child.stdin.close()
            except BrokenPipeError:
                pass
        errors = child.stderr.read()
        child.stderr.close()
        status = child.wait()
        if status or errors:
            raise RuntimeError(f"Benchmark failed: {arguments!r}: {status}: {errors!r}")


def summarize(values):
    quartiles = statistics.quantiles(values, n=4, method="inclusive")
    return {"median": statistics.median(values), "q1": quartiles[0], "q3": quartiles[2]}


def benchmark(options, root):
    binaries = {"gutil": executable(options.gutil), "gnu": executable(options.gnu)}
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    payload = bytes(range(256)) * (options.mib * 4096)
    source = root / "input"
    source.write_bytes(payload)
    digest = hashlib.sha256(payload).hexdigest()
    del payload
    generator = random.Random(42)  # nosec B311 -- Reproducible measurement order only.
    results = {}
    for piped in (False, True):
        for count in (0, 1, 4):
            names = [f"output-{index}" for index in range(count)]
            commands = {
                "gutil": [str(binaries["gutil"]), "--no-config", "--progress=never", "--jobs=1", "tee", *names],
                "gnu": [str(binaries["gnu"]), *names],
            }
            for label, arguments in commands.items():
                for name in names:
                    (root / name).unlink(missing_ok=True)
                with (root / "stdout-check").open("wb") as output:
                    invoke(arguments, root, source, environment, piped, output)
                for name in [*names, "stdout-check"]:
                    if hashlib.sha256((root / name).read_bytes()).hexdigest() != digest:
                        raise RuntimeError(f"Incorrect preflight output: {label}: {name}")
            samples = {label: [] for label in commands}
            for round_index in range(options.warmup + options.rounds):
                order = list(commands)
                generator.shuffle(order)
                for label in order:
                    for name in names:
                        (root / name).unlink(missing_ok=True)
                    started = time.perf_counter_ns()
                    invoke(commands[label], root, source, environment, piped)
                    # Include process creation and file opening in the measured interval.
                    elapsed = (time.perf_counter_ns() - started) / 1_000_000
                    for name in names:
                        if hashlib.sha256((root / name).read_bytes()).hexdigest() != digest:
                            raise RuntimeError(f"Incorrect output for {label}: {name}")
                    if round_index >= options.warmup:
                        samples[label].append(elapsed)
            reductions = [100 * (gnu - actual) / gnu for actual, gnu in zip(samples["gutil"], samples["gnu"], strict=True)]
            results[f"{'pipe' if piped else 'file'}-stdin/{count}-files"] = {
                "milliseconds": {label: summarize(values) for label, values in samples.items()},
                "paired_reduction_percent": summarize(reductions), "samples_ms": samples,
            }
    return {
        "machine": platform.platform(), "cpu": platform.processor(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "mib": options.mib, "rounds": options.rounds, "warmup": options.warmup, "seed": 42,
        "binaries": {label: {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()} for label, binary in binaries.items()},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/tee"))
    parser.add_argument("--mib", type=int, default=32)
    parser.add_argument("--rounds", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=4)
    options = parser.parse_args()
    if options.mib < 1 or options.rounds < 2 or options.warmup < 0:
        parser.error("positive MiB, at least two rounds, and nonnegative warmup required")
    with tempfile.TemporaryDirectory(prefix="gutil-tee-") as directory:
        print(json.dumps(benchmark(options, Path(directory)), indent=2))


if __name__ == "__main__":
    main()
