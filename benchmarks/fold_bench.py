#!/usr/bin/env python3
"""Compare gutil and GNU fold with verified interleaved measurements."""

import argparse
import hashlib
import json
import os
from pathlib import Path
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


def measure(binaries, source, destination):
    plain = (b"alpha beta gamma delta epsilon zeta eta theta iota kappa " * 4).rstrip() + b"\n"
    mixed = ("café monde 日本語 ひらがな 🧪 naïve espaços " * 4).rstrip().encode() + b"\n"
    cases = [
        ("ascii-columns", "C", ["-w72"], plain),
        ("ascii-spaces", "C", ["-sw72"], plain),
        ("utf8-columns", "C.UTF-8", ["-w72"], mixed),
        ("utf8-spaces", "C.UTF-8", ["-sw72"], mixed),
    ]
    randomizer = random.Random(477)  # nosec B311 -- Reproducible measurement order only.
    rounds, warmup = 12, 4
    results = {}
    for name, locale, flags, record in cases:
        source.write_bytes(record * (16 * 1024 * 1024 // len(record)))
        environment = dict(os.environ, LC_ALL=locale, TZ="UTC")
        environment.pop("POSIXLY_CORRECT", None)
        expected = None
        samples = {label: [] for label in binaries}
        for index in range(rounds + warmup):
            order = list(binaries)
            randomizer.shuffle(order)
            for label in order:
                command = [str(binaries[label])]
                if label != "gnu":
                    command.extend(["--no-config", "--progress=never", "--jobs=1", "fold"])
                command.extend([*flags, str(source)])
                with destination.open("wb") as output:
                    started = time.perf_counter_ns()
                    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
                    result = subprocess.run(command,  # nosec B603 -- Validated executable, shell=False.
                        env=environment, stdout=output, stderr=subprocess.PIPE, shell=False,
                    )
                    elapsed = (time.perf_counter_ns() - started) / 1_000_000
                if result.returncode or result.stderr:
                    raise RuntimeError(f"Unexpected status or stderr: {result!r}")
                with destination.open("rb") as output:
                    digest = hashlib.file_digest(output, "sha256").hexdigest()
                if expected is None:
                    expected = digest
                if digest != expected:
                    raise RuntimeError(f"Mismatched output from {label} on {name}")
                if index >= warmup:
                    samples[label].append(elapsed)
        results[name] = {
            "locale": locale,
            "flags": flags,
            "record": record.decode(),
            "input_bytes": source.stat().st_size,
            "output_sha256": expected,
            "samples_ms": samples,
            "median_ms": {label: statistics.median(values) for label, values in samples.items()},
        }
    return {
        "rounds": rounds,
        "warmup": warmup,
        "seed": 477,
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "binaries": {label: hashlib.sha256(path.read_bytes()).hexdigest() for label, path in binaries.items()},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/fold"))
    options = parser.parse_args()
    binaries = {"candidate": executable(options.candidate), "gnu": executable(options.gnu)}
    if options.baseline is not None:
        binaries["baseline"] = executable(options.baseline)
    with tempfile.TemporaryDirectory(prefix="gutil-fold-") as directory:
        root = Path(directory)
        print(json.dumps(measure(binaries, root / "input", root / "output"), indent=2))


if __name__ == "__main__":
    main()
