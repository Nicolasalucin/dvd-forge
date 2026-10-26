#!/usr/bin/env python3
"""Run bounded honggfuzz campaigns and retain logs, input hashes and summaries."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "iso9660": 8 << 20,
    "udf": 4 << 20,
    "bridge": 4 << 20,
    "roundtrip": (32 << 10) + 1,
    "iso_roundtrip": (32 << 10) + 16,
    "udf_roundtrip": 64 << 10,
    "media": 64 << 10,
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=10000)
    parser.add_argument("--output", type=Path)
    parser.add_argument("targets", nargs="*", choices=list(TARGETS))
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = (args.output or ROOT / "target" / "fuzz-runs" / f"{stamp}-{os.getpid()}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, HFUZZ_WORKSPACE=str(output / "workspace"))

    def run(command, name, run_env=env):
        print(f"{name}: {output / (name + '.log')}", flush=True)
        with (output / (name + ".log")).open("w") as log:
            return subprocess.run(command, cwd=ROOT, env=run_env, stdout=log, stderr=subprocess.STDOUT).returncode

    metadata = {"iterations_requested": args.iterations, "targets": {}, "seed_sha256": {}, "source_sha256": {}}
    for command in [["rustc", "--version"], ["cargo", "hfuzz", "version"]]:
        metadata[" ".join(command)] = subprocess.check_output(command, text=True).strip()
    for source in sorted((ROOT / "fuzz").rglob("*.rs")):
        if "target" not in source.relative_to(ROOT / "fuzz").parts:
            metadata["source_sha256"][str(source.relative_to(ROOT))] = hashlib.sha256(source.read_bytes()).hexdigest()
    for step in ["fuzz:check", "fuzz:seed"]:
        if run(["task", step], step.replace(":", "-")):
            raise SystemExit(f"{step} failed; retained evidence: {output}")
    for target in args.targets or TARGETS:
        corpus = ROOT / "fuzz" / "corpus" / target
        for seed in sorted(corpus.iterdir()):
            if seed.is_file():
                metadata["seed_sha256"][str(seed.relative_to(ROOT))] = hashlib.sha256(seed.read_bytes()).hexdigest()
        run_args = f"-n 1 -t 5 -N {args.iterations} -F {TARGETS[target]} --exit_upon_crash"
        target_env = dict(env, HFUZZ_RUN_ARGS=run_args)
        code = run(["task", "fuzz", "--", target], target, target_env)
        log = (output / (target + ".log")).read_text(errors="replace")
        summaries = re.findall(r"Summary iterations:(\d+) .*?crashes_count:(\d+) timeout_count:(\d+)[^\n]*", log)
        result = {"exit_code": code, "arguments": run_args}
        if summaries:
            iterations, crashes, timeouts = map(int, summaries[-1])
            result.update(iterations=iterations, crashes=crashes, timeouts=timeouts)
        result["passed"] = bool(summaries) and code == 0 and iterations >= args.iterations and crashes == timeouts == 0
        metadata["targets"][target] = result
        (output / "summary.json").write_text(json.dumps(metadata, indent=2) + "\n")
        if not result["passed"]:
            raise SystemExit(f"{target} failed; retained evidence: {output}")
    print(f"All campaigns passed; evidence: {output}", flush=True)


if __name__ == "__main__":
    main()
