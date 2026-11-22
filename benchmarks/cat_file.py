#!/usr/bin/env python3
"""Compare plain cat copies of a regular file on Linux."""

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
        raise ValueError(f"Expected an executable file: {resolved}")
    return resolved


def invoke(command, target, environment, verify_pipe=False):
    payload_hash = None
    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
    with subprocess.Popen(  # nosec B603 -- Validated executable, shell=False.
        command, env=environment, stdin=subprocess.DEVNULL, stdout=target,
        stderr=subprocess.PIPE, shell=False,
    ) as child:
        if child.stdout is not None:
            if verify_pipe:
                payload_hash = hashlib.file_digest(child.stdout, "sha256").hexdigest()
            else:
                buffer = bytearray(128 * 1024)
                while child.stdout.readinto(buffer):
                    pass
        errors = child.stderr.read()
        status = child.wait()
        if status or errors:
            raise RuntimeError(f"Unexpected benchmark result: {command!r}: {status}: {errors!r}")
    return payload_hash


def run_cases(commands, output, environment, expected, rounds, warmup):
    for label, command in commands.items():
        with output.open("wb") as target:
            invoke(command, target, environment)
        with output.open("rb") as result:
            digest = hashlib.file_digest(result, "sha256").hexdigest()
        if digest != expected or invoke(command, subprocess.PIPE, environment, verify_pipe=True) != expected:
            raise RuntimeError(f"Incorrect output from {label}")

    generator = random.Random(781)  # nosec B311 -- Reproducible measurement order only.
    results = {}
    for case in ("null", "file", "pipe"):
        samples = {label: [] for label in commands}
        for round_index in range(warmup + rounds):
            order = list(commands)
            generator.shuffle(order)
            for label in order:
                target = subprocess.DEVNULL if case == "null" else subprocess.PIPE
                if case == "file":
                    target = output.open("wb")
                try:
                    started = time.perf_counter_ns()
                    invoke(commands[label], target, environment)
                    elapsed = (time.perf_counter_ns() - started) / 1_000_000
                finally:
                    if case == "file":
                        target.close()
                if round_index >= warmup:
                    samples[label].append(elapsed)
        results[case] = {
            "median_ms": {label: statistics.median(values) for label, values in samples.items()},
            "samples_ms": samples,
        }
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/cat"))
    options = parser.parse_args()
    binaries = {
        "baseline": executable(options.baseline),
        "candidate": executable(options.candidate),
        "gnu": executable(options.gnu),
    }
    rounds, warmup = 12, 4
    flags = ["--no-config", "--progress=never", "--jobs=1", "cat"]
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    fixture_bytes = 64 * 1024 * 1024
    with tempfile.TemporaryDirectory(prefix="gutil-cat-file-") as directory:
        root = Path(directory)
        fixture = root / "input"
        output = root / "output"
        block = (b"x" * 63 + b"\n") * (1024 * 1024 // 64)
        with fixture.open("wb") as stream:
            for _ in range(fixture_bytes // len(block)):
                stream.write(block)
        with fixture.open("rb") as stream:
            expected = hashlib.file_digest(stream, "sha256").hexdigest()
        commands = {
            label: [str(path), *([] if label == "gnu" else flags), str(fixture)]
            for label, path in binaries.items()
        }
        results = run_cases(commands, output, environment, expected, rounds, warmup)

    print(json.dumps({
        "system": platform.platform(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rounds": rounds,
        "warmup": warmup,
        "seed": 781,
        "fixture_bytes": fixture_bytes,
        "output_sha256": expected,
        "binary_sha256": {label: hashlib.sha256(path.read_bytes()).hexdigest() for label, path in binaries.items()},
        "gutil_arguments": flags,
        "results": results,
    }, indent=2))


if __name__ == "__main__":
    main()
