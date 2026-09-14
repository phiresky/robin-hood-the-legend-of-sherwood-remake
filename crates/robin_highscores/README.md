# Highscores, replay verification, and deployment

This is the reference for the ranked service, browser frontend, ranked
authority authoring, VPS operations, Cloudflare deployment, and recovery.
Commands use repository-root paths unless a working directory is stated.
Source constants and checked-in typed validators own schema versions and
artifact contracts.

- [Service and API](#service-and-api)
- [Browser frontend and origin ownership](#browser-frontend-and-origin-ownership)
- [Multiplayer signatures](#multiplayer-signatures)
- [Contained replay verification](#contained-replay-verification)
- [Ranked authority authoring](#ranked-authority-authoring)
- [Operations](#operations)
- [Development and validation](#development-and-validation)

## Service and API

The API accepts complete runs and publishes only results reproduced by the
allowlisted verifier using exact official content, rules, build, and starting
campaign. **Server replay-verified** means the command stream reproduces the
result; it does not prove a human or an unmodified client generated it.

`robin-highscores-server` authenticates and bounds uploads, manages SQLite and
content-addressed replay/campaign objects, and serves boards.
`robin-highscores-worker` leases jobs and starts a fresh
`robin-replay-verifier` through digest-pinned `bwrap` and `prlimit`. All run as
the existing unprivileged `robinhood` account, with distinct systemd filesystem
restrictions. There is no broker, second service UID, or root worker.

A durable Ed25519 public key is the player identity; usernames are owner-signed
display metadata. Browser private keys remain non-extractable in the isolated
signer origin. All authenticated multiplayer participants must co-sign; a host
cannot impersonate guests.

Exactly one replay representation enters production:
`application/x-robin-rhrec+compact`, containing the current canonical bitcode
`ReplayFile` in its zstd/base64url envelope. Uploaded bytes become the verifier
input, retained object, and public download unchanged. JSONL is only a local
recorder/developer format. There is no public/private replay pair, alternate
media type, or historical-schema fallback. Both Demo and Full require the
complete exact starting campaign. Archive/Spellforge content is playable but
is not official Demo/Full ranked authority.

### HTTP contract

All API routes use `/api/v1`. Replay submission is multipart with exactly three
fields: UTF-8 JSON `submission`, the exact canonical replay, and the
exact starting campaign. The replay is read into a configured hard-bounded
buffer for a linear lexical scan before reservation; the campaign remains
streamed. Both are subject to one absolute upload deadline. Missing, duplicate,
swapped, truncated, media-type-mismatched, or digest/length-mismatched roles
fail closed.
The API treats the replay body as semantically opaque hostile
bytes: it performs only an allocation-free scan of the already bounded ASCII
envelope (`rhrec-`, the signed build's 12 lowercase hex characters, one
separator, and unpadded base64url text). It never base64-decodes, decompresses,
bitcode-decodes, or re-encodes the body. Only the resource-contained verifier
performs compressed framing, canonical bitcode, and current-header/schema
admission. Until that verifier returns a valid request-bound result, the
quarantined artifact has no public raw-digest or submission-ID download route.
After authenticating the first JSON field, the API reads and lexically checks
the replay field. A non-compact alternate format therefore creates no replay
object, queue row, upload reservation, or consumed challenge. The API then
atomically reserves the exact signed offer/envelope and consumes its one-use
challenge. Immediately before that transaction it rechecks both stores
and capacity using the signed exact replay/campaign lengths.
The remaining bounded upload slots reserve their configured maxima;
shared filesystems aggregate replay, campaign, multipart framing, SQLite WAL,
and one verifier-output allowance instead of counting the same free bytes
twice. SQLite enforces one cross-process ingestion lease. A
concurrent exact request receives `upload_in_progress` plus `Retry-After`; a
partial request releases that lease for an exact retry, and a crashed request
is recovered after lease expiry. Once both content-addressed artifacts are
durable, one transaction registers them, creates exactly one verifier job, and
commits the reservation. A completed retry returns the original lifecycle
without ingesting its artifact body again, while an uploaded-but-uncommitted
retry verifies the durable files and resumes the same canonical submission ID.
Noncommitted reservations have a bounded `upload_reservation_ttl_seconds`
window; expiry removes their otherwise-consumed challenge.
When admission is red, a fresh or artifact-writing retry consumes no challenge
and creates no reservation. Completed and active exact retries still return
their stable lifecycle or retry response, and an uploaded crash-recovery retry
may finish without ingesting bytes again.
The response is `202 Accepted`. Submission lifecycle is not public: an owner
first obtains a one-use challenge and then signs a private-status request for
that exact submission. Diagnostics, private participant instances, transcript,
campaign-chain locators, and request/result bindings remain private.

Replay downloads re-hash the content-addressed file before serving it.
Owner-signed deletion tombstones a run immediately; physical deletion happens
only after retention and when the digest has no live references. Abuse reports
create bounded moderation events and never hide or verify content
automatically.

Both normal API work and replay uploads have independent concurrency caps and
absolute request deadlines. Keep these bounds aligned with the process-level
memory/connection limits; raising the compiled safety ceilings is not an
alternative to capacity planning.

In the canonical proxy chain, the VPS firewall admits HTTPS only from
Cloudflare edges, nginx replaces `X-Forwarded-For` with Cloudflare's exact
`CF-Connecting-IP`, and the API trusts only nginx's loopback CIDR. A trusted
proxy request with no single canonical forwarded IP fails closed; forwarding
headers from every untrusted peer are ignored. nginx serves only `/api` paths
and returns 404 for other VPS-origin routes. `/healthz` and `/readyz` are
loopback/operator probes, not static-site routes.

Player boards accept the optional canonical `player_public_key` filter. The
typed `/api/v1/players/{public_key}/runs` resource returns snapshot-paginated
history and per-board personal bests. Cursor authentication binds the exact
key, query, visibility revision, and acceptance watermark.

Operator moderation routes are absent unless a private bearer-token file is
configured. The same list/review/action/audit functions are available through
`robin-highscores-admin`; every state transition is append-only audited.
`/healthz` is a non-database process-liveness check. `/readyz` is read-only:
it checks SQLite and storage/capacity admission. It does not create probe
objects; writable probes run at startup and on mutating admission paths.
Offer issuance uses the same gate plus replay/campaign store readiness. The verifier worker
rechecks capacity before every queue lease and again against the exact
authenticated final-campaign length before writing its object; it idles without
leasing while red. Garbage collection, backup, administrative repair, and
reservation recovery are deliberately outside this gate so they can restore
readiness. Authenticated operator status and Prometheus
text metrics live under `/api/v1/operator/operational-status` and
`/api/v1/operator/metrics`.

Immutable public BuildManifestV2, mission and campaign content, rules-config,
ruleset-manifest, competition, and policy JSON is served by digest under
`/api/v1/builds`, `/api/v1/content-manifests`,
`/api/v1/campaign-content-manifests`, `/api/v1/rules-configs`,
`/api/v1/ruleset-manifests`, `/api/v1/competitions`, and `/api/v1/policies`.
The separately cross-bound `/api/v1/published-rulesets/{digest}` response
carries mutable Active or Quarantined status with `Cache-Control: no-store`;
clients must preflight both documents. Full-campaign session proof and replay
routes are nested under `/api/v1/runs/{run_id}/sessions/{ordinal}`;
headquarters sessions are never exposed as standalone leaderboard runs. Run
and session campaign artifacts use the nested
`campaigns/{starting|final}` routes. Player profiles and histories are served
under `/api/v1/players/{public_key}` and `/api/v1/players/{public_key}/runs`.

Ranked simulation is authorized before frame zero. Hosts request a typed,
host-signed fresh-start grant at `/api/v1/fresh-run-preflight-grants`. A
campaign continuation instead uses
`/api/v1/campaign-continuation-preflight-grants`, whose request is signed by
both the next host and the immutable campaign controller and binds the exact
active predecessor result digest, starting campaign artifact, intended
durable roster, session identity, and ranked input tuple. The authority-signed
grant is embedded in session genesis. Missing, expired, substituted, or
wrong-authority grants are browse-only and cannot be repaired after the run.


### Database and object lifecycle

SQLx/SQLite migrations are explicit. Serving processes require the exact
current checksum-valid migration chain; do not edit `PRAGMA user_version`.
`ops/deploy.sh` applies append-only forward migrations with
`robin-highscores-admin migrate`; `ops/rollback.sh` refuses a target whose
supported schema differs from the live database. Pre-release dual-replay
databases are not upgrade inputs.

Artifacts are SHA-256-addressed regular files outside SQLite. Reconciliation
recovers interrupted inventory operations; retention-aware GC deletes only
unreferenced objects whose durable lifecycle permits it. Monitor WAL, object
stores, backups, inode availability and free space. The configured 1 GiB
readiness reserve does not account for space used by local backups.

## Browser frontend and origin ownership

The static leaderboard opens applicable boards automatically after a mission;
only eligible won runs may submit. Upload requires per-run consent unless the
player explicitly enables the default-off Always Submit Won Runs preference.
Board browsing, offers, signing, export and upload are frame-polled, so the
mission-end UI does not pause multiplayer. Failed or interrupted runs may
browse but cannot submit a successful score.

### Routes and static roots

The ordered authority is `wasm-www/deploy/public-routes.json`:

| Request | Owner | Source build root |
| --- | --- | --- |
| `robinhood.phiresky.xyz/api*` (including query strings) | VPS; null Worker script | none |
| `robinhood.phiresky.xyz/.well-known/acme-challenge/*` | nginx HTTP-01; null Worker script | none |
| `robinhood.phiresky.xyz/wasm/*` | `robinhood-runtime-assets` | `wasm-www/runtime-dist` |
| `robinhood.phiresky.xyz/datadirs/*` | separate `robinhood-datadir-assets` authority | `wasm-www/datadir-dist` |
| Remaining public-host requests | `robinhood-public-site` | `wasm-www/dist` |
| `identity.robinhood.phiresky.xyz/*` | `robinhood-identity-signer` | `wasm-www/signer-dist` |

There is no GitHub Pages, public binary mirror, Caddy-generated site, alternate
production API, or static fallback for API failures. nginx listens for the
Cloudflare-only API origin, forwards to `127.0.0.1:8787`, and returns 404 for
unowned origin paths. Exact `/healthz` and `/readyz` locations support controlled
origin probes. Its 130 MiB request ceiling must remain aligned with the compiled
multipart ceiling. The firewall must enforce the same reviewed Cloudflare edge
allowlist as nginx. Production CORS is disabled.

### Static closures

The public-site closure contains the landing page, leaderboard shell, viewer
shell, JS/CSS assets, public build/rules/policy/competition documents, and Demo
semantic content objects. The runtime closure contains the browser engine and
approved wasm-only runtime corpus below `/wasm`. Demo bytes are separately
authorized and manually deployed below `/datadirs`; they never enter the runtime
or site closures or a VPS release. The signer closure is a
separate, minimal page and bridge.

The wasm-bindgen engine JavaScript is also a closed module graph. Runtime
staging keeps the public browser identity client, removes the private identity
vault, and writes an exact sorted manifest entry for every retained imported
module with its relative path, byte length, and SHA-256. Verification derives
the graph again from `robin.js`; undeclared, orphaned, missing, substituted,
tampered, dynamic, or vault modules fail the release.

Full licensed content, Full semantic component documents, private projection
receipts, source-tree manifests, exporter/verifier executables, operator
configuration, campaign-state templates, and signing secrets are forbidden in
all four static roots.

Static inventories are digest-bound by `BuildManifestV2` and
`OfficialViewerBuildReportV2`. The `verify:public`, `verify:signer`,
`verify:runtime`, and `verify:datadir` scripts re-inventory the physical build
roots; a producer report cannot hide an extra file.

### Identity signer

Browser identities use a durable non-extractable WebCrypto Ed25519 key in the
separate signer origin's IndexedDB. The public site communicates with the signer
only through the typed `postMessage` contract, with exact origin, request id,
operation, key identity, challenge, and payload checks. The key owner can
change the display username by signing the API's short-lived username
challenge.

The signer does not expose raw key material, accept caller-selected origins,
perform arbitrary signatures, proxy API requests, or store replay/content
bytes. Native identity and browser identity use the same public protocol types;
they do not require a legacy re-key flow.

### Replay viewing

The viewer resolves immutable manifests by digest before downloading engine or
content assets. It checks the selected run's public proof, exact build, content,
rules configuration, replay digest and byte length. Demo content loads from the separately deployed
`/datadirs` authority named by the runtime. A Full replay may be listed and downloaded publicly,
but local playback requires the viewer to obtain a matching user-owned Full
installation; the service never publishes licensed Full bytes.

Playback is read-only and sandboxed from submission state. It does not mutate
the player's campaign, reuse a ranked offer, or silently submit anything. A
playback divergence is shown as a viewer failure and does not change the
server's immutable verification result.

### Caching and headers

Hashed JS/WASM/content objects and digest-addressed immutable manifests may use
long immutable caching. HTML entry points, mutable published-ruleset status,
leaderboard pages, player metadata, and API responses must not receive an
immutable cache policy. API responses are controlled by the VPS; the static
Worker must not cache them.

Headers are staged from:

- `wasm-www/deploy/public-headers.txt`;
- `wasm-www/deploy/runtime-headers.txt`;
- `wasm-www/deploy/datadir-headers.txt`;
- `wasm-www/deploy/signer-headers.txt`.

The public Content Security Policy admits only the exact same-origin API,
approved relay endpoints used by the browser game, and the isolated signer
frame/origin. Do not widen it for a development shortcut.

## Multiplayer signatures

Leaderboard submission signatures use one closed, fixed-size Ed25519 payload.
The native identity adapter, isolated browser signer, multiplayer transport, and
high-score server all use `LeaderboardCoSignRequestV1::signing_bytes`; none may
re-serialize the request or expose a raw signing operation.

### Fixed payload

The byte layout is:

1. `robinhood/leaderboards/1/co-sign-payload\0`
2. one purpose byte (`1` campaign continuation, `2` final submission)
3. the 32-byte ranked replay-session ID
4. SHA-256 of the complete validated `SubmissionOfferV1`
5. the 32-byte purpose-specific run digest

The offer digest binds the server's one-use upload challenge ID and nonce as
well as the signed session genesis. An instance is always reconstructed from
the authoritative offer as
`{purpose, replay_session_id, submission_offer_sha256}`. A client-provided
sequence or arbitrary instance is never accepted.

The purpose-specific run digests are intentionally separate:

- campaign continuation hashes the validated canonical
  `CampaignContinuationAuthorizationClaimV1` under
  `robinhood/leaderboards/1/campaign-continuation\0`; its helper also checks the
  claim against the continuation offer, predecessor, controller participant,
  session genesis, replay artifact, and starting campaign;
- final submission hashes the complete validated canonical
  `SubmissionEnvelopeV1` under `robinhood/leaderboards/1/submission\0`. This
  includes the exact replay artifact, starting campaign, participant transcript,
  authoritative offer, and the already completed controller authorization.

Consequently the two signing phases are ordered. If a continuation is present,
the immutable campaign controller signs it first. The resulting authorization
is inserted into the envelope, then every authenticated participant signs the
final submission request. Named and anonymous presentation does not alter this
requirement.

### Multiplayer and server checks

The host sends only the derived typed request. Each client transport accepts it
only when it exactly matches the request armed by the local mission-end flow and
the message belongs to the current authenticated Feature 39 connection. The
response echoes the exact instance; the host rejects wrong-purpose, stale,
cross-session, duplicate, and unexpected-key responses before collecting a
signature.

At upload, the server loads the stored authoritative offer, requires exact
offer equality, reconstructs the continuation and final requests with the same
protocol helpers, and verifies every Ed25519 signature over their fixed bytes.
The database consumes the upload challenge and signed replay-session genesis,
so replaying an otherwise valid signature cannot admit a second run.

`ParticipantSignatureV1` remains schema V1: its existing public key and
signature fields are sufficient because the server reconstructs all signed
bytes from the submitted envelope and stored offer. Changing the fixed layout,
purpose tags, digest domains, or instance derivation requires a new signing
contract version.

## Contained replay verification

### Content mounts

Official Demo and Full installations are separate typed content identities.
The operator supplies the selected raw edition as a read-only
mount plus its exact V2 source inventory and eight-component semantic bundle;
retail bytes are never included in the server image or repository. The
verifier re-inventories the mount without following symlinks, locally prepares
the selected subject, and requires its projection to match the exact bundle
before constructing ranked simulation. It receives normalized job semantics,
not the public viewer catalog or private projection-authority document.

A missing or mismatched official mount is a typed infrastructure failure, not a
gameplay rejection attributed to the replay. There is no fallback to another
install, Demo, overlay, zip, mod, or custom mission.

Mounts must be immutable for the entire worker lifetime. Read-only bind mounts
or read-only image layers close the check/use race between manifest validation
and asset loading.

### Required outer sandbox

In-process replay bounds are defense in depth, not an OS sandbox. Production
must run every job in a new rootless bubblewrap sandbox owned by the fixed
unprivileged `robinhood` account. The worker invokes `bwrap` and `prlimit`
directly as a structured argument vector, never through a shell, broker,
`systemd-run`, `sudo`, or polkit. The sandbox must have all of the following:

- fresh user, PID, IPC, UTS, cgroup, and network namespaces;
- a read-only, minimal executable view containing only the exact verifier and
  its required runtime files;
- the one selected Demo or Full raw-content root mounted read-only; the
  verifier re-inventories it against the exact source-tree manifest per job;
- four sealed read-only descriptors: request, exact canonical replay,
  normalized job configuration, and starting campaign;
- two precreated isolated writable descriptors: result and final campaign;
- no inherited listener, API, database, replay-store, secret-store, proxy, or
  other host descriptor;
- an empty home and environment, private temporary storage, no network
  interfaces, no host device access, and no writable host path;
- exact CPU-time, address-space, process-count, open-file, file-size, and
  zero-core limits applied by `prlimit`; and
- an independent worker-side wall timeout followed by complete process-tree
  termination and reaping.

The final worker configuration has exactly one mandatory
`[verifier_launcher]` table. Its fields are `bwrap_program`, `bwrap_sha256`,
`prlimit_program`, `prlimit_sha256`, `verifier_program`, `verifier_sha256`,
`wall_timeout_seconds`, `cpu_limit_seconds`,
`address_space_limit_bytes`, `process_limit`, `open_files_limit`,
`file_size_limit_bytes`, and `max_request_bytes`. Program paths are normalized
absolute paths; the verifier path names the binary inside the `authority`
release (`releases/<commit>/bin/robin-replay-verifier`). All limits are
positive values authored by `author-configs`.

Before every job the worker re-hashes all three programs, verifies every input
descriptor is the expected sealed regular object, checks the request size
before parsing, and constructs the fixed namespace/mount/limit argument list.
Missing user-namespace support, executable drift, an incomplete namespace or
mount closure, an unsealed input, or inability to apply a limit is an
infrastructure failure. There is no fallback launch mode.

The compact replay decoder still enforces its exact compressed,
decompressed, collection, and zstd-window limits inside the sandbox. An
address-space limit alone does not cap a decoder's history-window choice, and
a decompressed-byte limit alone does not cap all process allocation; both
layers are required.

### Admission order

The worker performs cheap, bounded checks before expensive ones:

1. Parse the small job document with a fixed byte limit and reject unknown
   fields or versions.
2. Authenticate every participant's signature over the canonical shared
   submission, including its exact artifact refs and separately co-signed
   lifecycle transcript. Named or anonymous public disclosure does not change
   this set.
3. Stream both supplied artifacts to EOF before decoding either, recompute byte
   counts and SHA-256 values, and distinguish retryable short/I/O failures from
   exact authenticated identity mismatches.
4. Resolve build, content, ruleset, difficulty, and starting-state identifiers
   exclusively through operator allowlists.
5. Validate the complete read-only content manifest.
6. Decode canonical compact bitcode with compressed size, base64, zstd window,
   decompressed-size, metadata, campaign, frame, and per-frame entry limits.
   Reject compact streaming-JSONL payloads by requiring deterministic bitcode
   re-encoding to reproduce the exact uploaded bytes. Validate dense indices
   and save/load references before engine construction.
7. Require the current replay schema's single-source ranked provenance and
   apply the command-policy scan.
8. Compare the entire canonical initial campaign blob with the individual
   template or exact server-issued predecessor blob, retain its exact
   `(edition, kind, rules_config_sha256)` authority, and require the embedded
   outer/nested restart `SimConfig` to equal the authenticated complete rules
   config. This includes nested restart/history state; field-wise partial
   comparisons are forbidden.
9. Run the renderer/audio/network-free engine with scripts enabled. Check each
   recorded state hash at its exact boundary and require the replay to end at a
   complete terminal success rather than merely exhausting input.
10. Derive all metrics and the final campaign from the single replay execution
   using checked arithmetic, then commit the two bound outputs.

Both output files begin empty. The final campaign remains empty for every
rejection and infrastructure failure and is accepted only beside a genuine
typed `Verified` result which binds its exact bytes. Exit status zero
means a typed verified-or-rejected result was written; any other status is an
infrastructure failure.

### Campaign chain

Individual-level boards accept only a server-installed canonical initial
campaign blob bound to its exact rules config. Full campaign genesis uses its
own exact-config canonical blob. Every continuation
must name the exact SHA-256 of the previous verifier-produced final blob and
the worker compares the new replay's complete initial campaign bytes with that
stored predecessor. Sherwood/HQ is a normal verified campaign session in this
chain; skipping it or reconstructing only inventory is invalid.

Campaign state affects substantially more than carried items: mission
availability/status, ARES and other script-visible values, roster and mission
team, character health/skills/capacities/ammunition, ransom/score/blazons,
relics, Sherwood production/occupants, prior achievements and immutable
attempts, generated names, selection/restart metadata, and mission-construction
RNG/config checkpoints. Exact whole-state chaining avoids omissions and makes
nested snapshot/history equivocation impossible.

Full Campaign completion is the Full edition's canonical 100% progression
trigger at `H12_Not_MP`. The optional later `SherwoodOutro` session does not
replace that predicate. Demo and rulesets without a Full Campaign board encode
completion as `NotOffered`.


The verifier production dependency closure excludes `robin_rs`, windowing,
rendering, audio/video and networking. A typed rejection is a job result;
crashes, limits, signals, timeouts or malformed/missing results are infrastructure
failures, retried only up to `max_verifier_attempts`, then exposed as
`verification_infrastructure`. Private diagnostics never become public proof.

## Ranked authority authoring

Ranked results are pinned to one verifier binary and one set of ranked
documents: `BuildManifestV2.verifier.sha256`, the rules configs, published
rulesets, competitions, campaign templates, official content bundles, and the
private verifier-job catalog. Together they form the **authority release**,
`~/.local/opt/robin-highscores/releases/<LIVE_COMMIT>`, which the `authority`
symlink names. The worker refuses to start if its configured verifier differs
from the pinned digest.

Author a new authority release only when a new build is deliberately
published: a new verifier, changed ranked content, new rules or rulesets, or
new competitions. **Routine service deploys never replace the verifier or the
`authority` release**: `ops/build-release.sh` without `--with-verifier` ships
only server, worker and admin, and `ops/deploy.sh` refuses to replace or prune
the authority target. All authoring commands write local files only; outputs
must be absent paths, and missing, extra, noncanonical or changed inputs fail.

### Tools and grant keys

```sh
cargo build -p robin_manifest_tool --bin robin-highscores-manifestctl
target/debug/robin-highscores-manifestctl probe-sandbox
cargo build -p robin_rs --example export_simulation_content \
  --no-default-features --features projection-export \
  --target x86_64-unknown-linux-musl --release
```

`projection-export` is absent from game, browser, Android, verifier and default
builds. On MUSL the build script supplies an empty `libdl.a` because MUSL
implements `dlopen` in libc. The projection authority hashes the resulting
exporter, so never rebuild it after authoring that authority. Inspection helpers:
`manifestctl hash FILE`, `validate-document --kind KIND FILE` and
`canonicalize --kind KIND IN OUT`.

The published rulesets and competitions pin the public halves of two Ed25519
grant keys, so on a fresh host create the secrets first. Each command reads only
its own path from the `--config` TOML and never prints secret bytes:

```sh
robin-highscores-admin --config bootstrap.toml initialize-cursor-key
robin-highscores-admin --config bootstrap.toml initialize-competition-run-grant-key
robin-highscores-admin --config bootstrap.toml initialize-run-preflight-grant-key
```

The last two print the public keys. The printable 32..128-byte
`moderation-bearer.token` has no initializer; create it by hand (mode `0400`).
All four live in `~/.local/share/robin-highscores/api-secrets/`.

### Build, content, rules and campaigns

1. Build and author the verifier and build documents. The `BuildDraftV2`
   names the verifier, browser inventories, and the typed tool authorities in
   `.github/tool-authorities/` (wasm-bindgen CLI, Node, pnpm; installed via
   `scripts/install_pinned_wasm_bindgen.sh`):

   ```sh
   crates/robin_highscores/ops/build-release.sh --with-verifier /absolute/out
   manifestctl author-build-v2 build-draft.json build-manifest-v2.json
   manifestctl author-projection-authority-v2 projection-draft.json projection-authority-v2.json
   manifestctl author-viewer-build-report-v2 build-manifest-v2.json viewer-build-report-v2.json
   ```

2. Produce the official Demo/Full projections. The `OfficialProjectionPlanV3`
   names the build, the wasm-bindgen/Binaryen/WABT authorities, projection
   authority, exporter, rules config, execution policy, core overlay and the
   Demo/Full loose and shipping source roots. Four sandboxed lanes run under
   `bwrap`/`prlimit`; loose and shipping results must be byte-identical:

   ```sh
   manifestctl author-official-content-v3 official-projections-v3.json /absolute/official-content
   ```

3. Generate rules and policies, the campaign-template matrix, and the
   published rulesets and competitions. The matrix plan is
   `{"official_content_authority", "rules_configs": [sorted paths], "schema_version": 1}`;
   the admission plan names the build, official content, policy inputs, both
   grant public keys and an explicit `competitions` array:

   ```sh
   cargo run -p robin_manifest_tool --example author_release_policy_inputs -- /absolute/policy-inputs
   manifestctl author-campaign-template-matrix-v1 matrix-plan.json /absolute/campaign-templates
   manifestctl validate-campaign-template-matrix-v1 matrix-plan.json /absolute/campaign-templates
   cargo run -p robin_manifest_tool --example author_release_admission_inputs -- \
     author release-admission-plan-v1.json /absolute/release-admission
   cargo run -p robin_manifest_tool --example author_release_admission_inputs -- \
     validate release-admission-plan-v1.json /absolute/release-admission
   ```

### Server/worker configs and verifier catalog

`scripts/release/author_leaderboard_release.py author-configs` derives the
admission-profile matrix from the manifest registry and authors the verifier
catalog through `manifestctl author-verifier-catalog-v1` and
`validate-verifier-catalog-v1`:

```sh
scripts/release/author_leaderboard_release.py author-configs \
  --source-commit "$AUTHORITY_COMMIT" \
  --plan /absolute/config-plan.json \
  --output /absolute/absent/config-authoring
```

`--source-commit` is the full 40-hex authority commit and must equal the
`BuildManifestV2` `source_commit`. The plan is JSON with exactly these keys:
`schema_version` (`3`), `manifest_directory`, `verifier_bundle_root`,
`manifest_tool` (a plain path to the executable), `bwrap_sha256`,
`prlimit_sha256`, `competition_run_grant_public_key`,
`run_preflight_grant_public_key`, `campaign_states` (the complete
rules-config-by-edition matrix of `{artifact, edition, kind,
rules_config_sha256, source}`), and `source_tree_manifests` (`{demo, full}`
paths named by their SHA-256). The output contains:

```text
config/{server.toml,worker.toml,api.env,worker.env}
private/verifier/operator-config/<catalog-sha256>
authoring/verifier-catalog-plan-v1.json
authoring-evidence.json
```

The rendered configs use fixed production paths. The server reads
`releases/<commit>/config/manifests` and
`releases/<commit>/private/campaign-states/<sha256>`. The worker reads
`~/.config/robin-highscores/server.toml`,
`releases/<commit>/private/verifier/operator-config/<sha256>`,
`releases/<commit>/private/source-tree-manifests-v2/<sha256>.json`, and
`releases/<commit>/bin/robin-replay-verifier`. Catalog entries point at
`releases/<commit>/private/verifier-bundles/...` and the fixed
`~/.local/share/robin-highscores/raw-content/{demo,full}` roots.

### Install an authority release

Copy the authored trees into `~/.local/opt/robin-highscores/releases/<commit>/`
at exactly those paths: `bin/robin-replay-verifier`, `config/manifests/`, and
`private/{campaign-states,source-tree-manifests-v2,verifier-bundles,verifier/operator-config}/`.
Copy the regular files (not symlinks) from `config/` to
`~/.config/robin-highscores/`. Then point
`ln -sfn releases/<commit> ~/.local/opt/robin-highscores/authority` and run
`ops/deploy.sh` with a service tarball, which migrates and restarts the
services. `deploy.sh` refuses a tarball whose commit directory is the authority
release, so run the services from a tarball built at a different commit.
Keep the previous authority directory until no rollback needs it.

## Operations

### Layout

```text
~/.local/opt/robin-highscores/
  releases/<LIVE_COMMIT>/   authority release (verifier, ranked content); never pruned
  releases/<commit>/        service releases: bin/, ops/, SOURCE_COMMIT, SHA256SUMS
  current -> releases/<commit>
  authority -> releases/<LIVE_COMMIT>
~/.config/robin-highscores/{server.toml,worker.toml,api.env,worker.env}
~/.config/systemd/user/     units copied from ops/systemd/
~/.local/share/robin-highscores/
  database/ replays/ campaign-states/ api-secrets/ raw-content/{demo,full}/ backups/
```

Everything runs as the unprivileged `robinhood` user in its lingering user
manager (`loginctl enable-linger robinhood`). `ops/systemd/` has
`robin-highscores-api.service`, `robin-highscores-worker.service`,
`robin-highscores.target`, `robin-highscores-backup.service` and
`robin-highscores-backup.timer`. The nginx origin in
`crates/robin_highscores/deploy/` is a one-time root install:
`nginx-robinhood-api.locations.conf` and `nginx-robinhood-cloudflare-only.conf`
go in `/etc/nginx/snippets/` as `robinhood-api.locations.conf` and
`robinhood-cloudflare-only.conf`, and `nginx-robinhood-api.vhost.conf` becomes
`/etc/nginx/sites-available/robinhood.phiresky.xyz`. Before the first
certificate exists, use the HTTP-only `nginx-robinhood-api.challenge.conf`
vhost, then run `certbot certonly --webroot --webroot-path /var/lib/letsencrypt`.
Keep the Cloudflare ranges and the firewall allowlist in sync. Licensed Demo
and Full trees are installed by hand, read-only, under `raw-content/`.

### Routine release

`scripts/release.sh` (root `README.md`, "Releasing") runs the steps below and
the Cloudflare frontend release as one command. Use `--server-only` for just
the service. It builds `ops/build-release.sh` inside the Debian 12 image from
`ops/release-image/Dockerfile`, in the detached worktree
`.worktrees/release-build` at `HEAD`, because the host's glibc 2.36 is older
than the development machine's. It copies the tarball to
`~/releases-incoming/`, checks its sha256, runs the tarball's own
`ops/deploy.sh`, and verifies `readyz` on the host and
`/api/v1/leaderboard-metadata` publicly. If a check fails it prints the
`rollback.sh` command for the previously live release. The sections below
document the individual steps.

### Build and deploy a service release

```sh
crates/robin_highscores/ops/build-release.sh [--with-verifier] [OUTPUT_DIR]
```

This runs `cargo build --locked --release -p robin_highscores --bins`, warns if
the tree has uncommitted changes, and writes
`robin-highscores-<commit>.tar.zst`. The tarball holds
`robin-highscores-<commit>/{bin/,ops/,SOURCE_COMMIT,SHA256SUMS}`.
Copy it to the VPS and run:

```sh
~/.local/opt/robin-highscores/current/ops/deploy.sh robin-highscores-<commit>.tar.zst
```

`deploy.sh` requires the `authority` symlink. It unpacks into
`releases/<commit>`, checks `sha256sum -c SHA256SUMS`, and stops the worker and
API. It then writes `admin snapshot-db` to
`backups/pre-deploy-<commit>-<time>/highscores.sqlite3`, runs `admin migrate`
and compares `database-schema-version` before and after. It swaps `current`
atomically, runs `daemon-reload`, starts `robin-highscores.target`, and waits
for `curl --retry 10` on `http://127.0.0.1:8787/readyz`. It then keeps 3
service releases (never the authority or current target) and the newest 5
pre-deploy snapshots. If a step fails after the stop, it acts as follows:

- **No migration ran:** it restores `current` and restarts the previous release.
- **A migration ran:** it restores `current` but leaves services **stopped**,
  because old binaries reject the newer schema. It prints the snapshot to
  restore: copy it over `database_path`, delete the `-wal`/`-shm` files, then
  `systemctl --user start robin-highscores.target`.

`deploy.sh` does not install unit files. If `ops/systemd/` changed, copy the
units into `~/.config/systemd/user/` first. Overrides:
`ROBIN_HIGHSCORES_ROOT`, `ROBIN_HIGHSCORES_SERVER_CONFIG`,
`ROBIN_HIGHSCORES_STATE`, `ROBIN_HIGHSCORES_HEALTH_URL`.

### Rollback

```sh
~/.local/opt/robin-highscores/current/ops/rollback.sh [commit]
```

The default target is the newest release that is neither current nor the
authority. Rollback refuses when the live `database-schema-version` differs
from the target's `supported-schema-version`. It also refuses releases that
predate that command, and the authority release. After a migration, restore
the schema first:

```sh
systemctl --user stop robin-highscores.target
cp ~/.local/share/robin-highscores/backups/pre-deploy-<commit>-<time>/highscores.sqlite3 \
  ~/.local/share/robin-highscores/database/highscores.sqlite3
rm -f ~/.local/share/robin-highscores/database/highscores.sqlite3-{wal,shm}
~/.local/opt/robin-highscores/current/ops/rollback.sh <previous-commit>
```

Writes after the snapshot are lost. Replay/campaign objects written later are
orphans that startup reconciliation and GC remove.

### Backups

`robin-highscores-backup.timer` runs daily at 02:15, with up to 45 minutes of
random delay and `Persistent=true`. It starts `current/ops/backup.sh`, which:

1. writes `admin snapshot-db` (`VACUUM INTO`, while services keep running);
2. hard-links `replays/` and `campaign-states/` (`cp -al`);
3. copies `api-secrets/` and `~/.config/robin-highscores` into
   `backups/<UTC stamp>/`;
4. keeps the newest 7.

The object trees may be slightly ahead of or behind the DB snapshot; startup
reconciliation tolerates both. Run it on demand with
`systemctl --user start robin-highscores-backup.service`. Off-host copy, run
from the other machine:

```sh
rsync -aH --delete robinhood@vps:.local/share/robin-highscores/backups/ ./robin-highscores-backups/
```

To restore:
1. Stop the target.
2. Copy `highscores.sqlite3` over the database (remove `-wal`/`-shm`).
3. Copy `replays/`, `campaign-states/` and `api-secrets/` back with `cp -a`.
4. Start the target.

### Frontend (Cloudflare)

```sh
CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_ZONE_ID=... CLOUDFLARE_API_TOKEN=... \
ROBINHOOD_WASM_BINDGEN=/abs/path/wasm-bindgen-0.2.128 \
  wasm-www/scripts/deploy-cloudflare.sh [--datadir] [--runtime]
```

The script needs Wrangler 4.131.1. It runs `verify:deployment-config`, builds
and verifies the public site and signer, and runs `verify:wrangler`. It deploys
`robinhood-identity-signer` then `robinhood-public-site`, reconciles routes with
`scripts/sync-cloudflare-routes.mjs --apply` and `--check`, and runs
`pnpm smoke:cloudflare`.
`--datadir` and `--runtime` also deploy the pre-assembled `datadir-dist`
(`assemble-datadir-corpus.mjs`) and `runtime-dist`
(`assemble-runtime-corpus.mjs`). Without them,
`ROBINHOOD_DATADIR_VERSION_ID` and `ROBINHOOD_RUNTIME_VERSION_ID` must name the
live Worker versions (`pnpm exec wrangler deployments list --config
deploy/wrangler-runtime.json`). The manual `deploy` job of
`.github/workflows/deploy-static-workers.yml` calls the same script. Rollback
per Worker:

```sh
pnpm --dir wasm-www exec wrangler rollback --config deploy/wrangler-<public|signer|runtime|datadir>.json [VERSION_ID]
```

### One-time host migration

This moves a host from the old sealed-release deploy to this layout. `ServerConfig`
is `deny_unknown_fields`, so configs, units and binaries change together.

1. Build the first simplified service tarball with `ops/build-release.sh` and
   copy it to the VPS.
2. `LIVE=$(readlink -f ~/.local/opt/robin-highscores/current)`.
3. Stop the old services:
   `systemctl --user disable --now robin-highscores.target robin-highscores-backup.timer`.
4. Take a cold copy of `database/`, `api-secrets/` and `$LIVE/config/`
   somewhere outside `~/.local/share/robin-highscores/backups/`.
5. Create the new config directory:
   ```sh
   mkdir -p ~/.config/robin-highscores
   grep -Ev '^(runtime_fence_directory|backup_authority_hmac_secret_path|backup_manifest_path|release_manifest_path|maximum_backup_age_hours) *=' \
     "$LIVE/config/highscores-server.toml" > ~/.config/robin-highscores/server.toml
   cp "$LIVE/config/highscores-worker.toml" ~/.config/robin-highscores/worker.toml
   cp "$LIVE/config/api.env" "$LIVE/config/worker.env" ~/.config/robin-highscores/
   ```
   The server now rejects those five keys. Worker fields are unchanged, but set
   its `server_config` to
   `"/home/robinhood/.config/robin-highscores/server.toml"`, since the old path
   still has the removed keys.
6. Remove the old units from `~/.config/systemd/user/`
   (`robin-highscores*.service`, `.target`, `.timer`). Also remove the
   `robin-highscores-*.service.d/50-robin-highscores-openat2-compat.conf`
   drop-ins; `RestrictSUIDSGID=no` is now in the base units. Install the new
   units from the tarball's `ops/systemd/`, then run
   `ln -sfn "$LIVE" ~/.local/opt/robin-highscores/authority`.
7. `chmod u+w ~/.local/opt/robin-highscores/releases` (old releases were
   mode `0550`).
8. Unpack the tarball somewhere temporary and run its `ops/deploy.sh` on the
   tarball.
9. `systemctl --user enable robin-highscores.target robin-highscores-backup.timer`,
   then `systemctl --user start robin-highscores-backup.service` once.
10. After a week without problems, remove the unused old state:
    `~/.local/share/robin-highscores/{runtime-fence,status}`,
    `~/.local/opt/robin-highscores/{activation.lock,incoming}`,
    `api-secrets/backup-authority-hmac.key`, and old `backups/backup-v4-*`
    generations plus `backups/.release-authorities-v2/`. Run
    `chmod -R u+w` first where directories are read-only.

## Development and validation

### Local development

Copy `crates/robin_highscores/highscores-server.example.toml` and
`highscores-worker.example.toml` to private absolute paths and configure the
exact authorities. Empty `admission_profiles` permits health/read access but
cannot issue offers. Build before starting a long-running process:

```sh
cargo build -p robin_highscores --bins
cargo build -p robin_replay_verifier --bin robin-replay-verifier
target/debug/robin-highscores-admin --config /absolute/highscores-server.toml migrate
# Run the following separately; each process remains running.
target/debug/robin-highscores-server --config /absolute/highscores-server.toml
target/debug/robin-highscores-worker --config /absolute/highscores-worker.toml
```

### Automated checks

```sh
cargo test -p robin_replay_format
cargo test -p robin_run_protocol
cargo test -p robin_highscores --features test-support
cargo test -p robin_highscores --features test-support --test router_e2e
cargo test -p robin_manifest_tool
bash crates/robin_highscores/ops/tests/deploy-rollback.sh
python3 -m unittest discover -s scripts/release -p 'test_author_leaderboard_release.py'
```

`ops/tests/deploy-rollback.sh` runs `deploy.sh` and `rollback.sh` against a
temporary HOME with stub `systemctl`, `curl` and binaries. It also runs as the
`ops_scripts` integration test.

From `wasm-www/`, use lockfile-exact dependencies and the checked-in scripts:

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm test:leaderboards
pnpm test:deployment
pnpm verify:public
pnpm verify:runtime-source
pnpm verify:runtime
pnpm verify:deployment-config
```

## Diagnostic reports

Crash and bug reports use `/api/v1/diagnostics` and the private operator endpoints.
See [Crash and bug reporting](../../docs/NEW_FEATURES.md#crash-and-bug-reporting)
for payload limits, retention, client behavior and migration requirements.
Diagnostics are included in the normal SQLite backup and maintenance fencing.
