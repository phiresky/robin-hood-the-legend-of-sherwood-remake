# Highscores service, replay verification, and deployment

This is the operator reference for the ranked leaderboard: what the service
does, the HTTP contract, how replays are verified, how to ship a verifier,
how boards are configured, and the routine VPS procedures. Commands use
repository-root paths unless stated otherwise.

- [Service and API](#service-and-api)
- [Sandboxed verification](#sandboxed-verification)
- [Verifier releases](#verifier-releases)
- [Boards](#boards)
- [Operations](#operations)
- [Development and tests](#development-and-validation)

## Service and API

The server receives a replay, the verifier resimulates it with our pinned
verifier build against raw game content, checks the recorded state hashes and
outcome, and scores it. **Server replay-verified** means the recorded command
stream reproduces the result; it does not prove that a human or an unmodified
client produced it.

- `robin-highscores-server` authenticates and bounds uploads, keeps SQLite and
  the content-addressed replay store, and serves boards.
- `robin-highscores-worker` leases queued submissions and runs a fresh
  `robin-replay-verifier` per job through `prlimit` and `bwrap`.
- `robin-highscores-admin` migrates, snapshots, bootstraps the cursor key and
  moderates.

All run as the unprivileged `robinhood` account under its systemd user
manager. A durable Ed25519 public key is the player identity; usernames are
owner-signed display metadata and must be registered before uploading.

Exactly one replay representation is ranked:
`application/x-robin-rhrec+compact`. Uploaded bytes are the verifier input,
the retained object and the public download, unchanged. The replay embeds its
starting campaign; there is no separate campaign upload.

### HTTP contract

All routes live under `/api/v1`.

| Route | Purpose |
| --- | --- |
| `GET leaderboard-metadata` | `LeaderboardMetadataV2`: boards from `server.toml` and the engine tick duration |
| `GET leaderboards` | flat `LeaderboardQueryV2` query → `LeaderboardPageV2` (authenticated cursors) |
| `GET runs/{id}`, `GET runs/{id}/replay` | `RunDetailV2`; exact compact replay bytes |
| `GET players/{key}`, `GET players/{key}/runs` | profile; `PlayerRunHistoryPageV2` with personal bests |
| `POST upload-challenges` | `UploadChallengeRequestV2` → one-use, expiring `UploadChallengeV1` bound to the key |
| `POST submissions` | multipart `submission` + `replay` → `202 SubmissionAcceptedV1` |
| `GET submissions/{id}/public-status` | minimal progress |
| `POST submission-owner-status-challenges`, `POST submissions/{id}/private-status` | owner-signed lifecycle, including rejection code |
| `POST username-challenges`, `PUT players/{key}/username` | signed rename |
| `POST deletion-challenges`, `POST deletion-requests` | owner-signed tombstone of a submission or run |
| `POST reports` | abuse report (per IP, key and target quotas) |
| `POST diagnostics`, `operator/*` | crash reports; bearer-token operator routes (absent without a token) |
| `/healthz`, `/readyz` | liveness; SQLite plus storage-capacity readiness |

`POST submissions` takes exactly two multipart fields in this order:

1. `submission`: `application/json` `SignedSubmissionV2`, decoded strictly
   (duplicate keys and unknown fields rejected), validated, and signature
   verified against `uploader_public_key` over
   `robinhood/leaderboards/2/submission\0` + the canonical submission.
2. `replay`: the compact replay with media type
   `application/x-robin-rhrec+compact`.

Before anything is reserved the API checks: the board exists, the mission is
on the board, the requested metrics are offered by the board, the challenge is
unexpired, the uploader has a registered username, the replay length and
SHA-256 equal the signed artifact, and the replay passes the allocation-free
lexical compact-transport scan (the API never base64/zstd/bitcode-decodes
hostile bytes). A red storage/capacity check consumes nothing.

One SQLite transaction then consumes the challenge (which must exist, be
unconsumed and have been issued to the uploader with the signed nonce and
expiry) and acquires an upload lease. A replay that is pending or was ever
accepted cannot be uploaded again by anyone; rejected and infrastructure-failed
replays may be retried. An exact retry of the same signed upload is idempotent:
it resumes the lease, reuses a durably stored replay, or returns the existing
lifecycle. Concurrent exact retries get `upload_in_progress` with
`Retry-After`. After the replay is stored, one transaction registers the
object, queues exactly one job and commits the reservation.

Replay downloads re-hash the stored object. Owner deletion tombstones
immediately; physical deletion waits for retention and for the digest to have
no live reference. Run detail, replays and history only show accepted runs of
boards that are currently configured.

nginx admits only Cloudflare, replaces `X-Forwarded-For` with
`CF-Connecting-IP`, and the API trusts only its loopback peer; a trusted peer
without exactly one canonical forwarded address fails closed.

### Database and objects

Migrations are explicit (`robin-highscores-admin migrate`, run by
`ops/deploy.sh`); serving processes require the exact checksum-valid chain.
Migration `0006` replaced all submission/run tables for protocol V2 and dropped
competitions, campaign chains, full-campaign aggregates, session geneses,
participant co-signing and campaign objects. Replays are SHA-256-addressed
files outside SQLite; startup and hourly maintenance reconcile the tree and
garbage-collect unreferenced objects after `orphan_replay_retention_hours`.
Rejected and failed submissions are tombstoned after
`rejected_replay_retention_hours`.

## Sandboxed verification

The worker leases one job, builds a `VerifierJobV2` from the board (edition,
mission, simulation policy, `allow_state_load`), the submission's replay
artifact, the edition's `resource_locale_root` and `[limits]`, writes the exact
job bytes and the stored replay into a private `0700` staging directory, and
runs:

```text
prlimit --core=0 --fsize=N --as=N --cpu=N --nofile=N --nproc=N -- \
  bwrap --unshare-all --unshare-user --disable-userns --assert-userns-disabled \
    --die-with-parent --new-session --cap-drop ALL --clearenv \
    --setenv PATH /run --setenv HOME /home/verifier --setenv TMPDIR /tmp \
    --setenv LANG C --setenv LC_ALL C --hostname robin-verifier \
    --size 16777216 --tmpfs / --proc /proc --dev /dev \
    --size 16777216 --tmpfs /run --size 16777216 --tmpfs /tmp --dir /var \
    --size 16777216 --tmpfs /var/tmp --size 16777216 --tmpfs /home \
    --dir /home/verifier --chmod 0700 /home/verifier --dir /run/robin-input \
    --ro-bind <verifier_program> /run/robin-verifier \
    --ro-bind <staging>/job.json /run/robin-input/job.json \
    --ro-bind <staging>/replay.rhrec /run/robin-input/replay.rhrec \
    --ro-bind <content.<edition>.root> /run/robin-content \
    --bind <staging>/result.json /run/robin-result.json \
    --remount-ro / --chdir /run -- \
  /run/robin-verifier --job /run/robin-input/job.json \
    --replay /run/robin-input/replay.rhrec --content-root /run/robin-content \
    --result /run/robin-result.json
```

The sandbox has fresh namespaces, no network, an empty environment and root,
the raw content tree read-only, and only the pre-created result file writable.
The worker kills the whole process group after `wall_timeout_seconds`.

Exit status zero means the result must be a strictly decoded, valid
`VerifierOutputV2` whose `job_sha256` equals the SHA-256 of the exact job
bytes and whose `replay_sha256` equals the stored replay. `Verified` publishes
the run (metrics, achievements, verifier-derived player counts, the uploader
named or anonymous per the signed disclosure); `Rejected` records the code
and detail; everything else — non-zero exit, timeout, missing or invalid
result, `FailedInfrastructure` — is retried up to `max_verifier_attempts` and
then marked `failed`, never as a rejection. A job queued under an older replay
schema is rejected as `unsupported_schema`.

The verifier itself checks, in order:

1. bounded compact decode with canonical re-encoding; Spellforge/archive
   content is rejected;
2. the replay schema and network protocol equal the verifier's compiled
   versions;
3. the board simulation policy admits the replay `SimConfig` (a fixed preset
   exactly, or any validated configuration for `any_config` boards);
4. the embedded starting campaign is an official fresh mission start built
   from the raw content's profiles, plus structural campaign validation;
5. command and input-provenance admission (automation, console, cheats and
   disallowed state loads are ineligible);
6. resimulation from raw content checking every recorded state hash, ending in
   a terminal success;
7. metrics and the compiled achievement catalog.

## Verifier releases

The `authority` symlink in `~/.local/opt/robin-highscores/` names the verifier
release: a directory with `bin/robin-replay-verifier` and `SOURCE_COMMIT`.
Recorded replays must resimulate identically, so build the verifier at the
commit of the **live web runtime**:

```sh
git worktree add .worktrees/verifier-build <live-web-commit>
.worktrees/verifier-build/crates/robin_highscores/ops/build-release.sh --with-verifier /absolute/out
# -> /absolute/out/robin-replay-verifier-<commit>.tar.zst (static musl)
```

On the VPS:

```sh
cd ~/.local/opt/robin-highscores/releases
tar -xf ~/releases-incoming/robin-replay-verifier-<commit>.tar.zst
mv robin-replay-verifier-<commit> <commit>
(cd <commit> && sha256sum -c SHA256SUMS)
ln -sfn releases/<commit> ~/.local/opt/robin-highscores/authority.new
mv -T ~/.local/opt/robin-highscores/authority.new ~/.local/opt/robin-highscores/authority
systemctl --user restart robin-highscores-worker.service
```

`worker.toml` launches `~/.local/opt/robin-highscores/authority/bin/robin-replay-verifier`,
so the symlink switch plus worker restart is the whole upgrade. Keep the
previous verifier directory until nothing needs to roll back to it (switch the
symlink back). `ops/deploy.sh` never prunes the authority target and refuses a
service release built at the same commit (the directories would collide).

## Boards

Boards are `[[boards]]` entries in `server.toml`, shaped exactly like
`BoardV2` and validated at startup (protocol validation, unique IDs, viewer
requirement matching the edition, fixed-policy labels matching the preset):

```toml
[[boards]]
board_id = "full-standard-normal"
display_name = "Full / Standard / Normal"
edition = "full"                       # demo | full
preset_id = "standard"
preset_name = "Standard"
difficulty_id = "normal"
difficulty_name = "Normal"
simulation_policy = { kind = "fixed", policy = { version = 1, preset = "standard", difficulty = "medium" } }
# or: simulation_policy = { kind = "any_config" }
allow_state_load = true
metrics = ["original_score", "fastest_success"]
viewer_content_requirement = "user_local_retail"   # bundled_demo for demo
missions = [{ mission_id = "H01_Lin_VL", display_name = "Official FULL H01_Lin_VL" }]
```

`crates/robin_highscores/ops/production/server.toml` is the complete
production configuration: Standard and Original × Easy/Normal/Hard plus an
`any_config` board for each edition; Full boards list the 38 field missions.
`ops/production/worker.toml` is the matching worker configuration. Removing a
board hides its runs; queued jobs for it fail after the retry policy.

### Raw content

The licensed Demo and Full trees are installed by hand, read-only, at
`~/.local/share/robin-highscores/raw-content/{demo,full}` and configured in
`worker.toml`:

```toml
[content.demo]
root = "/home/robinhood/.local/share/robin-highscores/raw-content/demo"
resource_locale_root = "1033"

[content.full]
root = "/home/robinhood/.local/share/robin-highscores/raw-content/full"
resource_locale_root = "2047"
```

Keep the trees immutable while the worker runs; they are never copied into a
release or the repository.

## Operations

### Layout

```text
~/.local/opt/robin-highscores/
  releases/<web-commit>/    verifier release: bin/robin-replay-verifier, SOURCE_COMMIT
  releases/<commit>/        service releases: bin/, ops/, SOURCE_COMMIT, SHA256SUMS
  current   -> releases/<commit>
  authority -> releases/<web-commit>
~/.config/robin-highscores/{server.toml,worker.toml,api.env,worker.env}
~/.config/systemd/user/     units copied from ops/systemd/
~/.local/share/robin-highscores/
  database/ replays/ api-secrets/ raw-content/{demo,full}/ backups/
```

`ops/systemd/` has `robin-highscores-api.service`,
`robin-highscores-worker.service`, `robin-highscores.target`,
`robin-highscores-backup.service` and `robin-highscores-backup.timer`
(lingering user manager: `loginctl enable-linger robinhood`). The nginx origin
in `crates/robin_highscores/deploy/` is a one-time root install.

Secrets in `api-secrets/`: create the cursor key with
`robin-highscores-admin --config server.toml initialize-cursor-key`; the
printable 32..128-byte `moderation-bearer.token` (mode `0400`) is created by
hand.

### Routine release

`scripts/release.sh` (root `README.md`, "Releasing") builds a service tarball
inside the Debian 12 image from `ops/release-image/Dockerfile`, copies it to
`~/releases-incoming/`, runs the tarball's own `ops/deploy.sh`, and checks
`readyz` and `/api/v1/leaderboard-metadata`. The steps:

```sh
crates/robin_highscores/ops/build-release.sh [OUTPUT_DIR]
~/.local/opt/robin-highscores/current/ops/deploy.sh robin-highscores-<commit>.tar.zst
```

`deploy.sh` requires the `authority` symlink with an executable verifier,
verifies `SHA256SUMS`, stops the worker and API, snapshots the database to
`backups/pre-deploy-<commit>-<time>/`, migrates, swaps `current`, starts
`robin-highscores.target` and waits for `readyz`. It keeps 3 service releases
(never `current` or the verifier release) and 5 pre-deploy snapshots. On
failure without a migration it restores and restarts the previous release;
after a migration it leaves services stopped and prints the snapshot to
restore. It does not install unit files: copy changed units from
`ops/systemd/` first. Overrides: `ROBIN_HIGHSCORES_ROOT`,
`ROBIN_HIGHSCORES_SERVER_CONFIG`, `ROBIN_HIGHSCORES_STATE`,
`ROBIN_HIGHSCORES_HEALTH_URL`.

### Rollback

```sh
~/.local/opt/robin-highscores/current/ops/rollback.sh [commit]
```

The default target is the newest release that is neither current nor the
verifier release. It refuses when the live schema differs from the target's
`supported-schema-version`; restore the matching snapshot first:

```sh
systemctl --user stop robin-highscores.target
cp ~/.local/share/robin-highscores/backups/pre-deploy-<commit>-<time>/highscores.sqlite3 \
  ~/.local/share/robin-highscores/database/highscores.sqlite3
rm -f ~/.local/share/robin-highscores/database/highscores.sqlite3-{wal,shm}
~/.local/opt/robin-highscores/current/ops/rollback.sh <previous-commit>
```

### Backups

`robin-highscores-backup.timer` runs `current/ops/backup.sh` daily: a
`snapshot-db` (`VACUUM INTO`) while services run, a hard-linked copy of
`replays/`, and copies of `api-secrets/` and the config directory into
`backups/<UTC stamp>/`, keeping the newest 7. Startup reconciliation tolerates
the object tree being slightly ahead of or behind the snapshot. Off-host:

```sh
rsync -aH --delete robinhood@vps:.local/share/robin-highscores/backups/ ./robin-highscores-backups/
```

To restore: stop the target, copy `highscores.sqlite3` over the database
(remove `-wal`/`-shm`), copy `replays/` and `api-secrets/` back with `cp -a`,
start the target.

## Development and validation

Copy `highscores-server.example.toml` and `highscores-worker.example.toml` to
private absolute paths, point `content.*.root` at local raw trees, and build
before starting long-running processes:

```sh
cargo build -p robin_highscores --bins
target/debug/robin-highscores-admin --config /abs/server.toml initialize-cursor-key
target/debug/robin-highscores-admin --config /abs/server.toml migrate
target/debug/robin-highscores-server --config /abs/server.toml
target/debug/robin-highscores-worker --config /abs/worker.toml
```

Checks:

```sh
cargo check -p robin_highscores --lib --bins --no-default-features
cargo test -p robin_highscores --features test-support     # unit, router_e2e, ops_scripts
bash crates/robin_highscores/ops/tests/deploy-rollback.sh
```

`router_e2e` drives the real router (challenge → signed upload → queue →
worker acceptance through the database → boards, history, deletion, reports).
`ops/tests/deploy-rollback.sh` exercises `deploy.sh` and `rollback.sh` against
a temporary HOME with stub `systemctl`, `curl` and binaries.

## Diagnostic reports

Crash and bug reports use `/api/v1/diagnostics` and the private operator
endpoints. See [Crash and bug reporting](../../docs/NEW_FEATURES.md#crash-and-bug-reporting)
for payload limits, retention and client behavior. Diagnostics are included in
the normal SQLite backup and maintenance fencing.
