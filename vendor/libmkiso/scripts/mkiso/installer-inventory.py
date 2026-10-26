#!/usr/bin/env python3
"""Hash exact original installer media without modifying or extracting inputs."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--entry", nargs=2, action="append", required=True,
                        metavar=("ID", "ISO"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    paths = [(entry, Path(source).resolve(strict=True)) for entry, source in args.entry]
    if len({entry for entry, _ in paths}) != len(paths):
        parser.error("duplicate entry ID")
    output = args.output.resolve()
    if output.exists():
        parser.error("output already exists; choose a new evidence path")
    if output in {path for _, path in paths}:
        parser.error("output must differ from every original media path")
    entries = []
    for entry, path in paths:
        if not path.is_file():
            parser.error(f"input is not a regular file: {path}")
        digest = hashlib.sha256()
        before = path.stat()
        with path.open("rb") as source:
            while chunk := source.read(8 * 1024 * 1024):
                digest.update(chunk)
        after = path.stat()
        identity = lambda st: (st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns)
        if identity(before) != identity(after):
            raise RuntimeError(f"source changed while hashing: {path}")
        entries.append({"entry_id": entry, "path": str(path),
                        "bytes": after.st_size, "sha256": digest.hexdigest()})
    with output.open("x") as report:
        json.dump({"schema": 1, "entries": entries}, report, indent=2)
        report.write("\n")


if __name__ == "__main__":
    main()
