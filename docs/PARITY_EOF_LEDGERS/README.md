# Permanent parity EOF ledgers

Each `*.snapshot` contains replay status keys that have reached exact EOF at
least once. Entries are permanent: the updater unions newly verified passes
with the existing file and never removes an entry.

Run `scripts/update_permanent_eof_ledgers.py` after an audit changes. It accepts
only status `0` records whose matching log contains exactly one anchored
`parity trace matched every recorded frame` marker. `summary.json` reports the
permanent count against the planned corpus denominator; seed 2m also reports
the currently captured count.

Schedulers must exclude every listed key. A replay is eligible only if it is
unseen or remains outside its group's permanent snapshot.

## Manual proof-loss entry

`schema16-seed2000000` Linux3/Profile003/Savegame_072/replay-006 reached exact
EOF once on 2026-08-21 with runner SHA-256
`d735b4aa05a6c23d732bf3d8b1e4c9b0fdd68496282f6a97cb1bd06e311b2ce6`.
The live autonomous watcher restored its stale status-1 file after the exact
log was moved into that watcher's output slot, destroying the sole log copy.
The key is therefore recorded directly in `seed2000000.snapshot` under the
permanent-EOF/no-rerun rule; no status-0 log was reconstructed and the replay
was not rerun.
