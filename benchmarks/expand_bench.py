#!/usr/bin/env python3
"""Compare gutil and GNU tab conversion with verified interleaved measurements."""

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


def scenarios(utility):
    if utility == "unexpand":
        plain = (b"        alpha   beta       gamma   delta                 end" * 4) + b"\n"
        mixed = ("        café   日本語        🧪          naïve      espaços" * 4).encode() + b"\n"
        return [
            ("ascii-leading", "C", [], plain),
            ("ascii-all", "C", ["-a"], plain),
            ("ascii-custom", "C", ["-t3,11,+5"], plain),
            ("utf8-all", "C.UTF-8", ["-a"], mixed),
        ]
    plain = (b"\t\t alpha\tbeta gamma\tdelta\tend " * 5).rstrip() + b"\n"
    mixed = ("\t café\t日本語\t🧪 naïve\t espaços " * 5).rstrip().encode() + b"\n"
    return [
        ("ascii-default", "C", [], plain),
        ("ascii-initial", "C", ["-i"], plain),
        ("ascii-custom", "C", ["-t3,11,+5"], plain),
        ("utf8-columns", "C.UTF-8", [], mixed),
    ]


def measure(utility, binaries, source, destination):
    randomizer = random.Random(477)  # nosec B311 -- Reproducible measurement order only.
    rounds, warmup = 12, 4
    results = {}
    for name, locale, flags, record in scenarios(utility):
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
                    command.extend(["--no-config", "--progress=never", "--jobs=1", utility])
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
    parser.add_argument("--utility", choices=("expand", "unexpand"), default="expand")
    parser.add_argument("--gnu", type=Path)
    options = parser.parse_args()
    reference = options.gnu or Path("/usr/bin") / options.utility
    binaries = {"candidate": executable(options.candidate), "gnu": executable(reference)}
    with tempfile.TemporaryDirectory(prefix="gutil-expand-") as directory:
        root = Path(directory)
        print(json.dumps(measure(options.utility, binaries, root / "input", root / "output"), indent=2))


if __name__ == "__main__":
    main()
