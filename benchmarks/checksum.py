#!/usr/bin/env python3
"""Compare saved checksum binaries with GNU using interleaved process timings."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess  # nosec B404 -- This harness launches user-selected benchmark executables.
import tempfile
import time

ALGORITHMS = (
    ("md5sum", "md5"), ("sha1sum", "sha1"), ("sha224sum", "sha224"),
    ("sha256sum", "sha256"), ("sha384sum", "sha384"), ("sha512sum", "sha512"),
)


def executable(path):
    resolved = path.resolve(strict=True)
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        raise ValueError(f"Expected a regular executable file: {resolved}")
    return resolved


def run_command(arguments, directory, environment, output):
    # Callers supply validated executables and a fixed checksum command allowlist.
    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
    return subprocess.run(arguments,  # nosec B603 -- Explicitly selected benchmark executable, shell=False.
        cwd=directory,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=output,
        stderr=subprocess.PIPE,
        check=False,
        shell=False,
    )


def summarize(values):
    quartiles = statistics.quantiles(values, n=4, method="inclusive")
    return {
        "median": statistics.median(values),
        "q1": quartiles[0],
        "q3": quartiles[2],
    }


def measure(arguments, directory, environment, expected):
    start = time.perf_counter_ns()
    result = run_command(arguments, directory, environment, subprocess.DEVNULL)
    elapsed = (time.perf_counter_ns() - start) / 1_000_000
    if result.returncode != expected:
        raise RuntimeError(f"{arguments!r}: {result.returncode}: {result.stderr!r}")
    return elapsed


def run(options, directory):
    binaries = {"before": executable(options.before), "after": executable(options.after)}
    references = {command: executable(options.gnu_dir / command) for command, _ in ALGORITHMS}
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    for variable in ("VERSION_CONTROL", "SIMPLE_BACKUP_SUFFIX", "POSIXLY_CORRECT"):
        environment.pop(variable, None)
    payload = bytes(range(256)) * (64 * 1024)
    (directory / "payload").write_bytes(payload)
    cases = [
        (
            f"{command}/16MiB", command, ["payload"],
            f"{getattr(hashlib, algorithm)(payload, usedforsecurity=False).hexdigest()}  payload\n".encode(), 0,
        )
        for command, algorithm in ALGORITHMS
    ]
    results = {}
    generator = random.Random(options.seed)  # nosec B311 -- Reproducible benchmark invocation order.
    for name, command, operands, output, status in cases:
        invocations = {
            label: [
                str(binary), "--no-config", "--progress=never", "--jobs=1",
                command, *operands,
            ]
            for label, binary in binaries.items()
        }
        invocations["gnu"] = [str(references[command]), *operands]
        for arguments in invocations.values():
            check = run_command(arguments, directory, environment, subprocess.PIPE)
            if check.returncode != status or check.stdout != output or check.stderr:
                raise RuntimeError(f"Verification failed: {arguments!r}: {check!r}")

        samples = {label: [] for label in invocations}
        for round_index in range(options.warmup + options.rounds):
            order = list(invocations)
            generator.shuffle(order)
            for label in order:
                elapsed = measure(invocations[label], directory, environment, status)
                if round_index >= options.warmup:
                    samples[label].append(elapsed)
        reductions = [
            100 * (before - after) / before
            for before, after in zip(samples["before"], samples["after"], strict=True)
        ]
        results[name] = {
            "milliseconds": {label: summarize(values) for label, values in samples.items()},
            "paired_reduction_percent": summarize(reductions),
            "after_wins": sum(value > 0 for value in reductions),
            "samples_ms": samples,
        }
    return {
        "machine": platform.platform(),
        "affinity": (
            sorted(os.sched_getaffinity(0))
            if hasattr(os, "sched_getaffinity") else None
        ),
        "rounds": options.rounds,
        "warmup": options.warmup,
        "seed": options.seed,
        "binaries": {
            label: {
                "path": str(binary),
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "bytes": binary.stat().st_size,
            }
            for label, binary in binaries.items()
        },
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("--gnu-dir", type=Path, default=Path("/usr/bin"))
    parser.add_argument("--rounds", type=int, default=100)
    parser.add_argument("--warmup", type=int, default=4)
    parser.add_argument("--seed", type=int, default=42)
    options = parser.parse_args()
    if options.rounds < 2 or options.warmup < 0:
        parser.error("--rounds must be at least 2 and --warmup must be nonnegative")
    try:
        with tempfile.TemporaryDirectory(prefix="gutil-checksum-") as temporary:
            print(json.dumps(run(options, Path(temporary)), indent=2))
    except (OSError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
