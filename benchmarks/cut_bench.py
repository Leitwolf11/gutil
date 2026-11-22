#!/usr/bin/env python3
"""Compare GNU and gutil record selection with verified output."""

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
    path = path.resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"Expected an executable file: {path}")
    return path


def measure(binaries, root):
    ascii_record = b"0000001:widget:2026-11-23:received:warehouse B:in stock\n"
    wide_record = "001:日本語é:🧪sample:received:naïve label\n".encode()
    cases = [
        ("ascii-bytes", "C", ["-b2-6,10-19"], ascii_record),
        ("ascii-fields", "C", ["-d:", "-f2,4"], ascii_record),
        ("utf8-characters", "C.UTF-8", ["-c5-12,15-27"], wide_record),
        ("utf8-delimiter", "C.UTF-8", ["-dé", "-f2"], wide_record),
    ]
    rng = random.Random(72313)  # nosec B311 -- Reproducible timing order.
    rounds, warmups = 12, 4
    results = {}
    source, destination = root / "input", root / "output"
    for name, locale, flags, record in cases:
        source.write_bytes(record * ((16 * 1024 * 1024) // len(record)))
        environment = dict(os.environ, LC_ALL=locale, TZ="UTC")
        environment.pop("POSIXLY_CORRECT", None)
        samples = {label: [] for label in binaries}
        expected = None
        for index in range(rounds + warmups):
            labels = list(binaries)
            rng.shuffle(labels)
            for label in labels:
                command = [str(binaries[label])]
                if label != "gnu":
                    command.extend(["--no-config", "--progress=never", "--jobs=1", "cut"])
                command.extend([*flags, str(source)])
                with destination.open("wb") as output:
                    started = time.perf_counter_ns()
                    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
                    result = subprocess.run(command,  # nosec B603 -- Validated executable, shell=False.
                        env=environment, stdout=output, stderr=subprocess.PIPE, shell=False,
                    )
                    elapsed = (time.perf_counter_ns() - started) / 1_000_000
                with destination.open("rb") as output:
                    digest = hashlib.file_digest(output, "sha256").hexdigest()
                if expected is None:
                    expected = digest
                if result.returncode or result.stderr or digest != expected:
                    raise RuntimeError(f"Unexpected result from {label} on {name}")
                if index >= warmups:
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
        "warmups": warmups,
        "seed": 72313,
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "binaries": {label: hashlib.sha256(path.read_bytes()).hexdigest() for label, path in binaries.items()},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/cut"))
    args = parser.parse_args()
    binaries = {"gutil": executable(args.gutil), "gnu": executable(args.gnu)}
    if args.baseline is not None:
        binaries["baseline"] = executable(args.baseline)
    with tempfile.TemporaryDirectory(prefix="gutil-cut-") as directory:
        print(json.dumps(measure(binaries, Path(directory)), indent=2))


if __name__ == "__main__":
    main()
