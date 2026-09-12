#!/usr/bin/env python3
"""Run an explicitly selected ignored Cargo test, refusing empty selections.

Listing uses the identical package, target, features and libtest selector as the
execution. Source-name scans cannot establish that a test is actually compiled.
Cargo output is replayed in full, including errors, before checking the listing.
"""
import subprocess
import sys


def main(arguments):
    if not arguments or "--" not in arguments:
        raise SystemExit("expected cargo test arguments followed by -- and libtest arguments")
    listing = subprocess.run(
        ["cargo", "test", *arguments, "--list"],
        stdout=subprocess.PIPE,
        text=True,
    )
    print(listing.stdout, end="", flush=True)
    if listing.returncode:
        return listing.returncode
    tests = [line.removesuffix(": test") for line in listing.stdout.splitlines()
             if line.endswith(": test")]
    if not tests:
        print("fixture gate selected no compiled tests; update the selector or feature configuration",
              file=sys.stderr)
        return 1
    if "--exact" in arguments and len(tests) != 1:
        print(f"exact fixture gate selected {len(tests)} tests, expected one", file=sys.stderr)
        return 1
    return subprocess.run(["cargo", "test", *arguments]).returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
