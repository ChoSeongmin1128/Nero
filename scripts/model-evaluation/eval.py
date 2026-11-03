#!/usr/bin/env python3
"""Explicit local evaluation CLI. plan/doctor do not load model weights."""
import argparse
import json
import os
from pathlib import Path
import signal
import sys

sys.dont_write_bytecode = True

from protocol import load_config, private_path, read_json
from report import write_report
from runner import cancel_from_signal, plan, run


def main():
    os.umask(0o077)
    signal.signal(signal.SIGTERM, cancel_from_signal)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("plan", "doctor", "run"):
        sub.add_parser(name).add_argument("--config", required=True)
    sub.add_parser("report").add_argument("--run-dir", required=True)
    args = parser.parse_args()
    if args.command == "report":
        directory = Path(args.run_dir).resolve()
        if not private_path(directory):
            raise ValueError("report output must remain private and untracked")
        print(json.dumps(write_report(read_json(directory / "results.json"), directory / "report"), indent=2))
        return 0
    config, profiles = load_config(args.config)
    if args.command in ("plan", "doctor"):
        rows = plan(config, profiles, doctor=args.command == "doctor")
        print(json.dumps(rows, ensure_ascii=False, indent=2))
        return 2 if any(row["status"] == "blocked" for row in rows) else 0
    if not private_path(args.config):
        raise ValueError("run config must be outside Git or explicitly ignored")
    directory, rows = run(config, profiles)
    print(json.dumps({"run_dir": str(directory), "statuses": {status: sum(row["status"] == status for row in rows) for status in sorted({row["status"] for row in rows})}}, ensure_ascii=False, indent=2))
    return 1 if any(row["status"] in ("failed", "unsupported", "not_scored") for row in rows) else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("Evaluation cancelled; completed receipts and partial outputs retained", file=sys.stderr)
        sys.exit(130)
    except (ValueError, OSError, KeyError, TypeError) as error:
        print("Evaluation error: " + str(error), file=sys.stderr)
        sys.exit(1)
