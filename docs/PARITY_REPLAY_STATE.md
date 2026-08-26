# Authoritative parity replay state

Parity state lives in one SQLite database on the remote worker:

```text
/srv/robinhood/parity-save-replays/replay-state.sqlite3
```

This database is the authority for scheduling and reporting. Do not maintain a
second writable copy, place it on NFS, or infer current state by counting audit
files. Local workers access the same database through commands sent over SSH
with `tmp/ssh_config`; they must not open a copied database locally.

`overview` is the operational handoff. It shows only active final-set corpora,
the explicitly selected authenticated runner, bitcode readiness, exact/failing/
untested counts, remaining work, database claims, externally running legacy
controllers, and concrete next actions. Permanent snapshot membership excludes
retired individual recordings even when their files remain inside a corpus.
Historical and retired corpora remain queryable with `summary`, `list-corpora`,
or `overview --json`, but do not clutter the default handoff.

Use `activate-corpus` or `import-final-snapshot` to place a corpus in the final
set. Use `retire-corpus --reason ...` to remove a corpus from operational
reporting without deleting its inventory or run history. Use
`set-current-runner` whenever the authenticated release bundle changes.
`merge-corpora` creates one canonical logical corpus from several physical
artifact roots. It moves authoritative membership and replay ownership while
retaining the retired source roots for path resolution and provenance. For
example, Seed0 is one 3,624-recording corpus assembled from the schema12,
schema14, and three replacement storage roots. Seed1m is likewise one
2,430-recording corpus even though its original capture, two replacements, and
one recapture live under three storage directories.

No database, lock, journal, backup, log, or generated helper may use `/tmp`.
Use `/srv/robinhood/parity-save-replays/` remotely and a repository-owned directory
such as `.agent-debug/replay-state/` locally.

## Identities and evidence

A replay has a stable corpus identity and corpus-relative logical path. Each
physical representation is a distinct artifact identified by its content
SHA-256 and encoding. A runner is identified by its authenticated bundle trust
SHA-256, together with the raw executable, wrapper, main-manifest, and
library-manifest hashes.

Every invocation creates a new run attempt. Attempts and their progress and
result rows are append-only; failures and reruns never replace earlier
evidence. A result records at least:

- the exact replay artifact and runner bundle;
- host, worker, command, start and finish times, and exit or signal status;
- highest consumed frame, matched-prefix frame count, and divergence frame
  when known;
- exact-EOF marker count and recorded frame total;
- input hashes before and after execution; and
- hashes and locations of the log, attestation, and result manifest.

Unknown progress is `NULL`, not zero. Exact EOF is valid only when the command
exits successfully, the anchored EOF marker occurs exactly once, input bytes
remain unchanged, and the matched-prefix count equals the recording's frame
total. Historical exact evidence remains visible even if a newer attempt
fails. Evidence from an older runner is not silently attributed to a newer
runner; reuse requires a separately recorded equivalence proof and scope.

Paths are locations, not identities. Moving an artifact or audit does not
change evidence as long as its content hash still verifies.

## Claims and leases

Replay and bitcode-conversion scheduling uses operational work and lease
tables separate from append-only evidence. A worker claims work in a short
`BEGIN IMMEDIATE` transaction, using a unique random claim token and bounded
lease. It may take a job only when no unexpired claim and no qualifying final
result exist. Expired leases may be reclaimed; attempt history is retained.

Conversion work is unique by source artifact, conversion protocol, and target
encoding. Once a verified conversion product exists, all runners reuse it.
The converting runner is still recorded on the conversion attempt. This
prevents local and remote workers from repeating protocol-2 conversion while
preserving its provenance.

Legacy controllers that operate on a whole corpus use `claim-corpus-work`,
`renew-corpus-work`, and `finish-corpus-work`. These bounded corpus leases are
shown by `overview`, so a local process cannot be accidentally duplicated by a
remote agent. A crashed controller's lease expires and can then be reclaimed;
completed, failed, and abandoned lease rows remain as coordination history.

Workers should keep write transactions short. Use WAL mode, foreign keys,
`synchronous=OFF`, and a 60-second busy timeout. The immutable audit evidence is
the recovery source if a host failure loses recent commits. Never hold a database transaction open
while converting or replaying; claim, commit, run, then append progress and
the final result in later transactions.

## Local access

For the global at-a-glance report:

```sh
ssh -F /home/phire/robinhood/tmp/ssh_config robin-worker -- \
  python3 /srv/robinhood/scripts/replay_state_db.py overview \
  /srv/robinhood/parity-save-replays/replay-state.sqlite3
```

Add `--json` for the complete machine-readable report.

Local orchestration performs claim and result operations on the remote host,
for example through a repository script invoked as:

```sh
ssh -F /home/phire/robinhood/tmp/ssh_config robin-worker -- \
  python3 /srv/robinhood/scripts/replay_state_db.py ...
```

Arguments and returned rows must use an unambiguous machine-readable format.
The remote command owns transactions and validation so quoting mistakes or a
lost SSH connection cannot partially publish evidence.

## Backups

Back up a live database only with SQLite's online backup API, the `.backup`
command, or `VACUUM INTO` to a new file under
`/srv/robinhood/parity-save-replays/replay-state-backups/`. Verify the backup with
`PRAGMA integrity_check`, hash it, and publish the hash alongside it. Never
copy only the main file while WAL mode is active, and never stage a backup in
`/tmp`.
