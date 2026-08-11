#!/usr/bin/env python3
"""Classify parity-sweep failures by their first authoritative boundary.

Reads an audit directory produced by scripts/run_parity_release_sweep.sh
(status/ + logs/) and prints machine-mergeable group summaries. Groups:

- pass: exit 0 with exactly one anchored exact-EOF marker
- integrity:eof-marker: exit 0 without exactly one exact-EOF marker, or an
  explicit integrity status emitted by the sweep runner
- rng: RNG cursor mismatch panic (grouped by first Rust RNG site list)
- unsupported-command: resolved input command the runner does not understand
- speech: resolved-speech FIFO/order boundary
- hang: timeout (exit 124)
- state: ordinary first-divergence state comparison, grouped by first logical
  field, and for actor.command additionally by original/rust command pair
- other-panic / other-error: anything else
"""

import collections
import json
import re
import sys
from pathlib import Path

EXACT_EOF_MARKER = "parity trace matched every recorded frame"
INTEGRITY_STATUS = "integrity-eof-marker"


def exact_eof_marker_count(text: str) -> int:
    return sum(line == EXACT_EOF_MARKER for line in text.splitlines())


audit = Path(sys.argv[1])
status_dir = audit / "status"
logs_dir = audit / "logs"

groups = collections.defaultdict(list)
detail = {}

for status_file in sorted(status_dir.glob("*.status")):
    key = status_file.stem
    code = status_file.read_text().strip()
    log = logs_dir / (key + ".log")
    if code == "124":
        groups["hang"].append(key)
        continue
    if code == "missing":
        groups["missing"].append(key)
        continue
    text = log.read_text(errors="replace") if log.exists() else ""
    if code == "0":
        marker_count = exact_eof_marker_count(text)
        if marker_count == 1:
            groups["pass"].append(key)
        else:
            groups["integrity:eof-marker"].append(key)
            detail[key] = f"exit 0 with {marker_count} exact-EOF markers"
        continue
    if code == INTEGRITY_STATUS:
        groups["integrity:eof-marker"].append(key)
        marker_count = exact_eof_marker_count(text)
        detail[key] = f"explicit integrity status; {marker_count} exact-EOF markers"
        continue
    m = re.search(r"unsupported resolved command[^\n]*|unknown \w*command[^\n]*", text)
    if m and "first parity divergence" not in text:
        cmd = re.search(r'"([a-z_]+)"', m.group(0))
        groups[f"unsupported-command:{cmd.group(1) if cmd else '?'}"].append(key)
        detail[key] = m.group(0)[:200]
        continue

    m = re.search(r"Rust consumed RNG draws \S+ at sites \[([^\]]*)\]", text)
    if m:
        sites = m.group(1)
        first_site = sites.split(",")[0].strip()
        groups[f"rng:{first_site}"].append(key)
        detail[key] = sites[:200]
        continue

    if re.search(r"speech|Speech", text) and "first parity divergence" not in text:
        groups["speech"].append(key)
        continue

    m = re.search(
        r"first parity divergence after frame (\d+) \((\d+) differences", text
    )
    if m:
        frame = m.group(1)
        fields = re.search(r"mismatch counts by logical field: (\{[^\n]*\})", text)
        first_field = None
        if fields:
            try:
                counts = json.loads(fields.group(1))
                first_field = min(counts, key=lambda k: -counts[k])
            except json.JSONDecodeError:
                pass
        # Use the comparator's own ordering: the first listed "first X:" line.
        m2 = re.search(r"first ([a-z_.]+): (\S+)[^\n]*", text)
        listed_field = m2.group(1) if m2 else (first_field or "?")
        if listed_field == "actor.command" or "actor.command" in text:
            pair = re.search(
                r"first actor\.command: [^:]*: original=(\w+) rust=(\w+)", text
            )
            if pair:
                groups[f"state:actor.command:{pair.group(1)}->{pair.group(2)}"].append(
                    key
                )
                detail[key] = f"frame {frame}"
                continue
        groups[f"state:{listed_field}"].append(key)
        detail[key] = f"frame {frame}"
        continue

    if "panicked" in text:
        pm = re.search(r"panicked at ([^\n:]+:\d+)", text)
        groups[f"other-panic:{pm.group(1) if pm else '?'}"].append(key)
        detail[key] = text[text.find("panicked") : text.find("panicked") + 200]
        continue

    groups[f"other-error:exit-{code}"].append(key)

total = sum(len(v) for v in groups.values())
print(f"total classified: {total}")
for name, members in sorted(groups.items(), key=lambda kv: -len(kv[1])):
    print(f"\n## {name} ({len(members)})")
    for k in members[:10]:
        extra = detail.get(k, "")
        print(f"  {k}  {extra}")
    if len(members) > 10:
        print(f"  ... and {len(members) - 10} more")

# Full machine-readable dump for downstream agents.
out = audit / "classification.json"
out.write_text(json.dumps({k: v for k, v in groups.items()}, indent=1))
print(f"\nwrote {out}")
