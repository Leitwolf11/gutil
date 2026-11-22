#!/usr/bin/env python3
"""Compare C-locale wc word counting with an earlier gutil and GNU wc."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import statistics
import subprocess  # nosec B404 -- Explicit benchmark binaries, never a shell.
import tempfile
import time

def valid_executable(path):
    result = path.resolve(strict=True)
    if not result.is_file() or not os.access(result, os.X_OK):
        raise ValueError(f"Expected an executable file: {result}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/wc"))
    parser.add_argument("--utf8", action="store_true", help="Measure an ASCII file and a mixed UTF-8 file in C.UTF-8")
    options = parser.parse_args()
    binaries = {
        "baseline": valid_executable(options.baseline),
        "candidate": valid_executable(options.candidate),
        "gnu": valid_executable(options.gnu),
    }
    rounds, warmup = 12, 3
    randomizer = random.Random(42)  # nosec B311 -- Reproducible invocation order only.
    results = {}
    with tempfile.TemporaryDirectory(prefix="gutil-wc-c-bench-") as directory:
        fixture = Path(directory) / "input"
        pattern = b"alpha beta\tword\vlast\n multiple   spaces \r\n"
        size = 16 * 1024 * 1024
        cases = [
            ("words", ["-w"], "C", pattern),
            ("default", [], "C", pattern),
            ("utf8-words", ["-w"], "C.UTF-8", pattern),
        ]
        if options.utf8:
            cases = [
                ("utf8-ascii-words", ["-w"], "C.UTF-8", pattern),
                ("utf8-ascii-words-chars", ["-wm"], "C.UTF-8", pattern),
                ("utf8-mixed-words", ["-w"], "C.UTF-8", "café mundo\t日本語 autre\n".encode()),
            ]
        for case, flags, locale, fixture_pattern in cases:
            fixture.write_bytes((fixture_pattern * (size // len(fixture_pattern) + 1))[:size])
            if options.utf8:
                # Truncate the final sequence cleanly while keeping the file size fixed.
                data = fixture.read_bytes().decode("utf-8", errors="ignore").encode()
                fixture.write_bytes(data.ljust(size, b" "))
            environment = dict(os.environ, LC_ALL=locale, TZ="UTC")
            samples = {label: [] for label in binaries}
            expected = None
            for round_index in range(rounds + warmup):
                order = list(binaries)
                randomizer.shuffle(order)
                for label in order:
                    command = [str(binaries[label])]
                    if label != "gnu":
                        command += ["--no-config", "--progress=never", "--jobs=1", "wc"]
                    command += [*flags, str(fixture)]
                    start = time.perf_counter_ns()
                    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
                    result = subprocess.run(command,  # nosec B603 -- Validated executable, shell=False.
                        env=environment, capture_output=True, check=True, shell=False,
                    )
                    duration = (time.perf_counter_ns() - start) / 1_000_000
                    if result.stderr:
                        raise RuntimeError(f"Unexpected stderr for {label}: {result.stderr!r}")
                    if expected is None:
                        expected = result.stdout
                    if result.stdout != expected:
                        raise RuntimeError(f"Mismatched {case} output from {label}")
                    if round_index >= warmup:
                        samples[label].append(duration)
            results[case] = {
                "flags": flags,
                "locale": locale,
                "fixture_pattern": fixture_pattern.decode(),
                "samples_ms": samples,
                "median_ms": {label: statistics.median(times) for label, times in samples.items()},
            }
        print(json.dumps({
            "rounds": rounds,
            "warmup": warmup,
            "seed": 42,
            "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
            "fixture_bytes": size,
            "binaries": {k: hashlib.sha256(p.read_bytes()).hexdigest() for k,p in binaries.items()},
            "results": results,
        }, indent=2))


if __name__ == "__main__":
    main()
