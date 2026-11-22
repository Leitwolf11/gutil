#!/usr/bin/env python3
"""Compare GNU and gutil topological ordering with verified output."""

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


def write_graph(path, pairs):
    with path.open("w", encoding="ascii", newline="\n") as output:
        for source, target in pairs:
            output.write(f"node-{source:07d} node-{target:07d}\n")


def measure(gutil, gnu, root):
    count = 120_000
    names = list(range(count))
    rng = random.Random(4172)  # nosec B311 -- Reproducible fixtures and timing order.
    rng.shuffle(names)
    order = list(range(1, count))
    rng.shuffle(order)
    cases = {
        "chain": ((names[i - 1], names[i]) for i in range(1, count)),
        "fan-out": ((names[0], names[i]) for i in order),
        "independent": ((i, i) for i in names),
        "binary-tree": ((names[(i - 1) // 2], names[i]) for i in order),
    }
    commands = {
        "gutil": [str(gutil), "--no-config", "--progress=never", "--jobs=1", "tsort"],
        "gnu": [str(gnu)],
    }
    environment = dict(os.environ, LC_ALL="C", TZ="UTC")
    environment.pop("POSIXLY_CORRECT", None)
    rounds, warmups = 12, 4
    results = {}
    for name, pairs in cases.items():
        source, destination = root / "input", root / "output"
        write_graph(source, pairs)
        samples = {label: [] for label in commands}
        expected = None
        for index in range(rounds + warmups):
            labels = list(commands)
            rng.shuffle(labels)
            for label in labels:
                command = [*commands[label], str(source)]
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
            "nodes": count,
            "relations": count if name == "independent" else count - 1,
            "input_bytes": source.stat().st_size,
            "output_sha256": expected,
            "samples_ms": samples,
            "median_ms": {label: statistics.median(values) for label, values in samples.items()},
        }
    return {
        "rounds": rounds,
        "warmups": warmups,
        "seed": 4172,
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "binaries": {label: hashlib.sha256(path.read_bytes()).hexdigest()
                     for label, path in (("gutil", gutil), ("gnu", gnu))},
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gutil", type=Path)
    parser.add_argument("--gnu", type=Path, default=Path("/usr/bin/tsort"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="gutil-tsort-") as directory:
        print(json.dumps(measure(executable(args.gutil), executable(args.gnu), Path(directory)), indent=2))


if __name__ == "__main__":
    main()
