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
