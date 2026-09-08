# Highscores, replay verification, and deployment

This is the shared reference for the ranked service, browser frontend, release
publication, VPS installation, Cloudflare deployment, and recovery. Commands
use repository-root paths unless a working directory is stated; paths inside a
sealed VPS release are relative to that release. Replace illustrative paths
and `EXPECTED_*`/`APPROVED_*` values with the exact reviewed inputs. Source
constants and checked-in typed validators own schema versions and artifact
contracts; historical release numbers are not deployment authority.

This file is the only source guide. Release authoring copies it to
`deploy/README.md` and generates two short section links for the fixed
`VPS_RELEASE_INSTALL.md` and `BACKUP_RESTORE.md` bundle roles. Source documentation is not evidence of current
production state: use the selected release and retained deployment receipts.

- [Service and API](#service-and-api)
- [Browser frontend and origin ownership](#browser-frontend-and-origin-ownership)
- [Multiplayer signatures](#multiplayer-signatures)
- [Contained replay verification](#contained-replay-verification)
- [Release authoring](#release-authoring)
- [VPS bundle contract](#vps-bundle-contract)
- [VPS installation and rollback](#vps-installation-and-rollback)
- [Backup and disaster recovery](#backup-and-disaster-recovery)
- [Cloudflare deployment and rollback](#cloudflare-deployment-and-rollback)
- [Protected GitHub workflows](#protected-github-workflows)
- [Host compatibility overlay](#host-compatibility-overlay)
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
challenge. Immediately before that transaction it rechecks both stores,
backup freshness, and capacity using the signed exact replay/campaign lengths.
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
it checks SQLite, storage/capacity admission and authenticated backup status;
production remains unready until the first verified backup. It does not
create probe objects or continuously reverify protected backup payloads. Offer issuance uses the same gate. The verifier worker
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
Authenticated deployment can apply a supported append-only forward migration;
automatic rollback requires the current release, target, and live database to
share one schema. Pre-release dual-replay databases are not upgrade inputs.

Artifacts are SHA-256-addressed regular files outside SQLite. Reconciliation
recovers interrupted inventory operations; retention-aware GC deletes only
unreferenced objects whose durable lifecycle permits it. Monitor WAL, object
stores, backups, inode availability and free space. The configured 1 GiB
readiness reserve does not replace the typed backup-capacity calculation.

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
authorized and manually deployed below `/datadirs`; they never enter runtime,
site, publication, or VPS bundles. The signer closure is a
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

Every release inventory is digest-bound by `BuildManifestV2`,
`OfficialViewerBuildReportV2`, and the operator publication lock. The
publication assembler independently inventories physical files; a producer
report cannot hide an extra file.

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
The operator supplies the selected receipt-approved raw edition as a read-only
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
- the one selected Demo or Full raw-content root mounted read-only only after
  its complete source-tree manifest has been revalidated;
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
absolute paths; the verifier path names the exact immutable source-commit
release. All limits are positive, release-authorized values.

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

## Release authoring

All authoring commands produce local immutable handoffs; none uploads,
activates services, initializes secrets, or changes Cloudflare. Use one clean
source checkout and exact reviewed tools. Outputs use create-new semantics;
missing, extra, noncanonical or changed inputs abort the transaction.

The order is: bind source and tools; produce build/projection authority;
author official content, rules, campaigns and admissions; derive configs and
catalog; assemble and validate PublicationV3; then create the VPS and
Cloudflare handoffs. Generate the VPS grant keys before finalizing the
manifests that bind their public keys (see installation below).

### Bind the final source

Build every source-bound artifact from one clean checkout. Record the reviewed
values and verify them against that checkout:

```sh
scripts/release/author_leaderboard_release.py author-source-authority \
  --repo /absolute/final/source \
  --source-commit "$SOURCE_COMMIT" \
  --source-tree-sha1 "$SOURCE_TREE_SHA1" \
  --cargo-lock-sha256 "$CARGO_LOCK_SHA256" \
  --database-schema-version "$DATABASE_SCHEMA_VERSION" \
  --output /absolute/handoff/source-authority-v1.json

scripts/release/author_leaderboard_release.py verify-context \
  --repo /absolute/final/source \
  --source-authority /absolute/handoff/source-authority-v1.json
```

The authority is compact canonical JSON. Verification requires exact `HEAD`,
tree, `Cargo.lock`, and the shared
`HIGHSCORES_DATABASE_SCHEMA_VERSION`; tracked modifications fail closed.
Untracked private operator evidence is outside the source closure.

### Build the tools

```sh
cargo build -p robin_manifest_tool --bin robin-highscores-manifestctl
target/debug/robin-highscores-manifestctl --help
target/debug/robin-highscores-manifestctl probe-sandbox
```

The exact native projection exporter is a separate, native-only build:

```sh
cargo build -p robin_rs --example export_simulation_content \
  --no-default-features --features projection-export \
  --target x86_64-unknown-linux-musl --release
```

`projection-export` is absent from game, browser, Android, ordinary verifier,
and default developer builds. On MUSL only, the build script supplies a local
empty `libdl.a` compatibility archive because MUSL implements `dlopen` in libc.
No host library or dynamically linked fallback is admitted.

The projection-authority manifest hashes the exact resulting executable. Never
rebuild it after authoring that authority.

### Author the build authorities

Draft documents contain explicit paths and expected authorities; the tool
rehashes their contents and emits canonical documents:

```sh
target/debug/robin-highscores-manifestctl author-build-v2 \
  public-build-draft.json build-manifest-v2.json
target/debug/robin-highscores-manifestctl author-projection-authority-v2 \
  private-projection-authority-draft.json projection-authority-v2.json
target/debug/robin-highscores-manifestctl author-viewer-build-report-v2 \
  build-manifest-v2.json viewer-build-report-v2.json
```

The public build binds the verifier executable and complete browser-engine,
public-static, and identity-signer inventories. The private projection
authority cross-binds the public build and exact exporter. Toolchain and lock
authorities are explicit. Source schema identities are taken from current Rust
protocol/engine constants; release plans do not carry hand-maintained replay,
save, network, shipping-datadir, content, or localization version literals.

`BuildDraftV2.wasm_bindgen_cli` is required and has exactly this shape (the
path may be relative to the draft file):

```json
{
  "version": "0.2.127",
  "authority_document": ".github/tool-authorities/wasm-bindgen-cli-v0.2.127.json"
}
```

The authority document pins the upstream `wasm-bindgen-cli` crate URL, package
digest and byte length, exact version, role, and installed executable path.
Both the public viewer engine and isolated identity signer must carry its
canonical digest. The build author refuses omission, a different path target,
role substitution, version drift, or modified package facts. The browser
runtime and identity-signer workflows both install that same package through
`scripts/install_pinned_wasm_bindgen.sh`; deriving a CLI version from
`Cargo.lock` or finding an ambient `wasm-bindgen` on `PATH` is not an accepted
release build.

Node and pnpm use the same closed model. `BuildDraftV2.node` must name
`.github/tool-authorities/node-v24.19.0.json`; `BuildDraftV2.pnpm` must name
`.github/tool-authorities/pnpm-v12.3.4.json`. These typed documents bind the
exact upstream archive URL, archive digest and length, selected executable
path, and selected executable digest and length. A generic JSON file carrying
only a version, an ambient executable on `PATH`, or a role substitution is not
an authority. Checked-in authority documents may have one final newline;
their manifest identity is always the canonical JSON bytes without it.

Useful inspection commands are:

```sh
target/debug/robin-highscores-manifestctl hash artifact.bin
target/debug/robin-highscores-manifestctl validate-document \
  --kind build-manifest-v2 build-manifest-v2.json
target/debug/robin-highscores-manifestctl canonicalize \
  --kind competition competition-draft.json competition.json
```

Strict JSON parsing rejects duplicate and unknown fields. Canonical validation
rejects wrong ordering, zero identities, unsupported schemas, and changed
artifacts.

### Produce official Demo and Full projections

An `OfficialProjectionPlanV3` supplies absolute normalized paths to:

- the public build, wasm-bindgen CLI authority, Binaryen authority, WABT
  authority, private projection authority, and exact exporter;
- the exact rules configuration and execution policy;
- the engine-owned core overlay source;
- separate Demo and Full loose/native source roots;
- separate Demo and Full shipping source roots.

The plan does not supply subject lists, component paths, shipping split lists,
source manifests, receipts, or already-generated projections. Those are derived
from the current typed engine and protocol.

The three required tool-authority fields are:

```json
{
  "wasm_bindgen_cli_authority": "/operator/authorities/wasm-bindgen-cli-v0.2.127.json",
  "binaryen_wasm_opt_authority": "/operator/authorities/binaryen-wasm-opt-v132.json",
  "wabt_wasm_strip_authority": "/operator/authorities/wabt-wasm-strip-v1.0.41.json"
}
```

There is no older two-authority plan form. The CLI fails before launching a
projection if any field is omitted, if a path is absent, symlinked, or not
normalized, or if the typed authority does not match the role/version/digest
bound by the public build.

Run the transaction into an absent output path:

```sh
target/debug/robin-highscores-manifestctl author-official-content-v3 \
  official-projections-v3.json "$PWD/operator-output/official-content"
```

The tool performs four isolated lanes: Demo loose, Demo shipping, Full loose,
and Full shipping. It derives exact source closures, copies them into fresh
read-only mounts, and launches the exact exporter under pinned `/usr/bin/bwrap`
and `/usr/bin/prlimit`. Networking, host environment, home, user profiles,
mods, caches, and unrelated files are absent.

Shipping source selection decodes the current `ShippingDatadir` and selects
`Data/datadir.bin` plus the union of mission, character RHS, character audio,
and saved-world split payloads named by its typed fields. The historical
`ShippingDatadirV10` enum label is frozen protocol vocabulary; the authoring
implementation does not assert archive schema 10. Locale validation uses the
archive's current canonical locale model and exact source LCID (`1033` Demo,
`2047` Full).

For every protocol subject the exporter prepares mission input once and writes
exactly the canonical component order declared by
`SIMULATION_CONTENT_COMPONENT_ORDER_V1`. The loose and shipping results for an
edition must be byte-identical. After export, all sources, authorities, and the
executable are hashed again. Only a complete four-lane matrix is atomically
renamed into place.

For additional assurance, author twice from independently materialized inputs
and compare the resulting content, campaign, receipt, and matrix digests.

### Author campaign templates and the verifier catalog

Generate the current source-derived ranked rules, projection execution policy,
and four immutable admission policies into an absent staging directory:

```sh
cargo run -p robin_manifest_tool --example author_release_policy_inputs -- \
  /operator/staging/policy-inputs
```

This authors Standard and Original Parity rules for Easy, Medium, and Hard.
Only won missions are accepted, every run requires a complete compact-bitcode
replay and server resimulation, and the projection policy is the exact current
Standard/Medium configuration. The command derives complete `SimConfig`
objects from engine policy code; it does not read defaults from an operator
JSON file.

Campaign state is current-schema canonical bitcode, not an opaque save chosen
by the operator. Author both editions for every admitted rules configuration
as one atomic matrix. The canonical plan contains only schema version 1, the
complete Plan-V3 authority root, and a non-empty, lexically sorted and unique
list of rules-config paths:

```sh
printf '%s' '{"official_content_authority":"/operator/official-content","rules_configs":["/operator/policy-inputs/original-parity-easy.json","/operator/policy-inputs/original-parity-hard.json","/operator/policy-inputs/original-parity-medium.json","/operator/policy-inputs/standard-easy.json","/operator/policy-inputs/standard-hard.json","/operator/policy-inputs/standard-medium.json"],"schema_version":1}' \
  > campaign-template-matrix-plan-v1.json
target/debug/robin-highscores-manifestctl author-campaign-template-matrix-v1 \
  campaign-template-matrix-plan-v1.json "$PWD/campaign-template-matrix-v1"
target/debug/robin-highscores-manifestctl validate-campaign-template-matrix-v1 \
  campaign-template-matrix-plan-v1.json "$PWD/campaign-template-matrix-v1"
```

Demo is exactly `Campaign::from_profiles` admitted as `individual_template`;
Full is the same fresh current campaign admitted as `full_campaign_genesis`.
Authoring validates the complete four-lane authority once, requires every
subject in an edition to bind identical Profiles bytes, strictly decodes that
authenticated component into the exact runtime `ProfileManager`, and emits
`templates/<rules-digest>/<demo|full>.bitcode` plus the canonical
`campaign-template-matrix-v1.json` index. Validation repeats the full authority
admission once and exact-rederives every byte and path. It does not accept a
standalone component, raw unnormalized shipping profile set, old save,
continuation, restart snapshot, partial configuration, or merely decodable
campaign. Every template artifact uses the protocol's canonical
`application/x-robin-campaign+bitcode` media type.

The private verifier-job catalog is also derived rather than hand assembled:

```sh
target/debug/robin-highscores-manifestctl author-verifier-catalog-v1 \
  verifier-catalog-plan-v1.json final-server.toml verifier-catalog-v1.json
target/debug/robin-highscores-manifestctl validate-verifier-catalog-v1 \
  verifier-catalog-plan-v1.json final-server.toml verifier-catalog-v1.json
```

The plan is canonical JSON containing exactly `schema_version: 1`, the full
40-hex source commit, the digest-addressed manifest directory, the private
verifier-bundle root, and the complete list of campaign-template source files.
Authoring re-derives every profile x allowed-scope x ordinary/competition
route from the final strict TOML, re-hashes every selected manifest, bundle,
policy, and campaign, and pins production paths below the immutable source
commit. A duplicate, omission, unreferenced campaign, mutable path, unresolved
sentinel value, symlink, or route substitution fails the release.


### Published rulesets and competitions

`PublishedRulesetV1` and `CompetitionManifestV1` are generated, never written
by hand. First generate `policy-inputs` with
`author_release_policy_inputs`, then prepare one canonical compact JSON plan
with these required fields:

- `schema_version`: `1`
- `build_manifest`: exact canonical `BuildManifestV2`
- `official_content_authority`: completed `author-official-content-v3` output
- `policy_inputs_directory`: exact, unmodified policy-input tree
- `run_preflight_grant_public_key`: real 32-byte public key as 64 lowercase hex
- `competition_run_grant_public_key`: a different real public key
- `competitions`: explicit array, including when it is empty

Each competition entry supplies its stable ID/version, display text, edition,
typed ranked simulation policy, typed mission/full-campaign subject, metric,
pinned seed, participant composition, and exact start/end Unix milliseconds.
Entries must be ordered by ID/version and use unique IDs. Mission categories
are `individual_level` for Demo and `campaign` for Full. Schedules begin no
earlier than 2020 and may last at most 366 days.

Author and independently revalidate the absent output tree:

```sh
cargo run -p robin_manifest_tool --example author_release_admission_inputs -- \
  author release-admission-plan-v1.json release-admission-v1
cargo run -p robin_manifest_tool --example author_release_admission_inputs -- \
  validate release-admission-plan-v1.json release-admission-v1
```

The result contains six digest-addressed rules configs, four policies, twelve
active published rulesets, explicit competition manifests, and
`release-admission-index-v1.json`. Demo rulesets offer individual mission
boards. Full rulesets offer campaign-mission and complete-campaign boards.
All build/content/config/policy/campaign-state/key identities are derived from
the admitted inputs and regenerated during validation; missing, extra, or
substituted files fail closed.
### Author server, worker, and host configuration

The config plan is compact canonical JSON with exactly these fields:

```json
{
  "bwrap_sha256": "APPROVED_SHA256",
  "campaign_states": [
    {
      "artifact": {
        "byte_length": 1,
        "media_type": "application/x-robin-campaign+bitcode",
        "sha256": "APPROVED_SHA256"
      },
      "edition": "demo",
      "kind": "individual_template",
      "rules_config_sha256": "APPROVED_SHA256",
      "source": "/absolute/campaign-state"
    }
  ],
  "competition_run_grant_public_key": "APPROVED_PUBLIC_KEY",
  "manifest_directory": "/absolute/prepublication/manifests",
  "manifest_tool": {
    "artifact": {
      "byte_length": 1,
      "media_type": "application/x-executable",
      "sha256": "APPROVED_SHA256"
    },
    "source": "/absolute/bin/robin-highscores-manifestctl"
  },
  "prlimit_sha256": "APPROVED_SHA256",
  "run_preflight_grant_public_key": "APPROVED_PUBLIC_KEY",
  "schema_version": 2,
  "source_tree_manifests": {
    "demo": "/absolute/source-tree-manifests/DEMO_SHA256.json",
    "full": "/absolute/source-tree-manifests/FULL_SHA256.json"
  },
  "verifier_bundle_root": "/absolute/prepublication/verifier-bundles"
}
```

`campaign_states` must contain the complete rules-config-by-edition matrix;
the abbreviated example is not sufficient for a real release. All digests and
public keys are explicit reviewed inputs.

```sh
scripts/release/author_leaderboard_release.py author-configs \
  --repo /absolute/final/source \
  --source-authority /absolute/handoff/source-authority-v1.json \
  --plan /absolute/handoff/config-authoring-plan-v2.json \
  --output /absolute/absent/config-authoring-v2
```

The command derives the complete admission-profile matrix from canonical
manifest registries, authors the private verifier catalog twice, compares the
bytes, validates it with the exact manifest tool, and renders tracked host
files with the exact commit. The host closure includes these three mandatory
preactivation-gate roles in the same frozen order used by
`VpsReleasePlanV2`:

```text
real_runtime_fence_release_gate deploy/tests/real-runtime-fence-release-gate.sh 0550
real_runtime_fence_harness      deploy/tests/real-runtime-fence-e2e.py           0550
real_runtime_fence_selftest     deploy/tests/real-runtime-fence-e2e-selftest.py  0550
```

The sources are the corresponding tracked files below
`crates/robin_highscores/deploy/tests/`. They are re-read from the exact source
commit, byte-compared with the checkout, and sealed executable. A missing,
modified, reordered, non-executable, or substituted gate is an authoring or
plan failure. In particular, server configuration always uses:

```text
runtime_fence_directory = .../runtime-fence
backup_authority_hmac_secret_path = .../api-secrets/backup-authority-hmac.key
backup_manifest_path = .../status/backup-status.json
release_manifest_path = .../releases/COMMIT/vps-release-manifest-v2.json
maximum_backup_age_hours = 32
```

The fifth backup-authority key is only named. Its bytes are neither accepted
nor emitted by release authoring.

### Assemble a deployment publication

An `OperatorPublicationPlanV3` supplies the admitted content authority, exact
build draft, viewer build report, verifier operator configuration, campaign
state templates, rules configurations, policies, published rulesets,
competitions, and a typed fresh/update/status-transition/rollback decision.

```sh
target/debug/robin-highscores-manifestctl assemble-publication-v3 \
  publication-plan-v3.json "$PWD/operator-output/release"
target/debug/robin-highscores-manifestctl validate-publication-v3 \
  "$PWD/operator-output/release"
```

The release layout is:

```text
backend/
  manifests/                   public API registries by digest
private/
  official-content-authority/  complete exact Plan-V3 tree, including bundles
  verifier/bin/                pinned native verifier by digest
  verifier/operator-config/    pinned private job catalog by digest
  campaign-states/             exact ranked input templates by digest
cloudflare-public/             public site + Demo viewer/content manifests
cloudflare-identity-signer/    isolated signer static closure
deployment/exposure-v3.json    exact origin and route contract
backend/publication-v3.json
publication-manifest-v3.json
publication-manifest-v3.sha256
publication-lock-v3.json
publication-lock-v3.sha256
```

The complete Plan-V3 authority is copied once as regular singleton files and
fully revalidated offline before any nested artifact is trusted. The admitted
wasm-bindgen, Binaryen, and WABT authority documents live under
`private/official-content-authority/manifests/build-tool-authorities/` by
canonical digest. They are therefore part of every publication lock and are
reloaded by the offline publication validator; removing or substituting any of
the three invalidates the release. The validator also exact-rederives every
campaign template from the authenticated edition profiles and rules config.

Datadir objects are not normal release output. The public shell and WASM
runtime contain only the exact separately installed Demo datadir authority
(version, canonical inventory digest, native-content digest, and stable URL),
never datadir bytes. The dedicated datadir publication is manually assembled
and validated from canonical Demo converter output and deployed only when that
authority changes. Full retail bytes have no public datadir authority. Normal
publication and rollback must fail if the named Demo authority is absent or
substituted; they must not republish it.

The V1 datadir authority's `source_commit` and `cargo_lock_sha256` identify the
producer that created that immutable corpus. They do not need to equal a later
consumer's `BuildManifestV2`: a new game build may reuse an unchanged deployed
datadir. The authority remains pinned byte-for-byte, its deployment receipt
must name the same authority digest and exactly repeat its inventory, producer,
Worker, route, origin, and Demo content identity, and the current game's build,
origin inventories, checkout, and `Cargo.lock` remain independently exact.

### Author VpsReleasePlanV2

After assembling and independently approving PublicationV3, author the plan
and exact local-to-remote upload map. `musl-binary-authority-v2.json` is a
compact canonical producer handoff with exact
`schema_version,source_commit,source_tree_sha1,cargo_lock_sha256,binaries`
fields. `binaries` contains the five canonical V2 roles in order, each as
`{artifact,role,source}`. The author re-hashes every binary; it does not mint
or silently replace the producer's approved identities.

```sh
scripts/release/author_leaderboard_release.py author-vps-plan \
  --repo /absolute/final/source \
  --source-authority /absolute/handoff/source-authority-v1.json \
  --publication /absolute/publication-v3 \
  --approved-publication-lock-sha256 "$PUBLICATION_LOCK_SHA256" \
  --binary-authority /absolute/handoff/musl-binary-authority-v2.json \
  --configs /absolute/config-authoring-v2 \
  --remote-source-root \
    "/home/robinhood/.local/opt/robin-highscores/incoming/.sources-$SOURCE_COMMIT" \
  --remote-demo-raw-root \
    /home/robinhood/.local/share/robin-highscores/raw-content/demo \
  --remote-full-raw-root \
    /home/robinhood/.local/share/robin-highscores/raw-content/full \
  --output /absolute/absent/vps-plan-v2
```

The exact source-built manifest tool validates PublicationV3 before the plan
is emitted. `vps-release-plan-v2.json` uses schema 2 and V2 role order;
`release-source-handoff-v2.json` describes every binary/config/host upload and
the PublicationV3 root. Its nineteen host entries include all three sealed
runtime-fence gate executables above; the plan author rejects mode drift before
emitting their remote sources. The separately installed immutable Demo and
Full raw trees are declarations only and are never copied into a release.

Run `assemble-vps-release-v2` and `validate-vps-release-v2` on the VPS only
after the upload tree has been independently checked. The candidate manifest
is `vps-release-manifest-v2.json` and binds the shared compiled database
schema. After candidate validation, the bundled
`deploy/tests/real-runtime-fence-release-gate.sh` is the mandatory final
preactivation gate. This authoring tool neither runs that gate nor activates
the candidate.

### Materialize and assemble the Cloudflare handoff

PublicationV3 first crosses the Rust-owned materialization boundary:

```sh
scripts/release/author_leaderboard_release.py materialize-cloudflare \
  --repo /absolute/final/source \
  --source-authority /absolute/handoff/source-authority-v1.json \
  --manifest-tool /absolute/bin/robin-highscores-manifestctl \
  --manifest-tool-sha256 "$MANIFEST_TOOL_SHA256" \
  --publication /absolute/publication-v3 \
  --approved-publication-lock-sha256 "$PUBLICATION_LOCK_SHA256" \
  --output /absolute/absent/cloudflare-materialization-v1
```

Record and independently approve the printed materialization receipt digest.
Then use the accepted OperatorBundleV2 six-argument contract. The JavaScript
operator receives no Publication or manifest-tool path:

```sh
scripts/release/author_leaderboard_release.py assemble-cloudflare \
  --repo /absolute/final/source \
  --source-authority /absolute/handoff/source-authority-v1.json \
  --node /absolute/approved/node \
  --node-sha256 "$NODE_EXECUTABLE_SHA256" \
  --materialization /absolute/cloudflare-materialization-v1 \
  --wasm-static /absolute/approved/wasm-static \
  --approved-materialization-receipt-sha256 "$MATERIALIZATION_RECEIPT_SHA256" \
  --approved-runtime-inventory-sha256 "$RUNTIME_INVENTORY_SHA256" \
  --output /absolute/absent/cloudflare-deployment-bundle-v2
```

The underlying command is exactly:

```text
operator-deployment-bundle.mjs assemble MATERIALIZATION WASM_STATIC OUTPUT APPROVED_MATERIALIZATION_RECEIPT_SHA256 APPROVED_RUNTIME_INVENTORY_SHA256 REPO_ROOT
```

Cloudflare preflight and live publication remain separate reviewed operations
described in [Cloudflare deployment and rollback](#cloudflare-deployment-and-rollback).

## VPS bundle contract

`assemble-vps-release-v2` packages one validated PublicationV3 and reviewed
host inputs into an immutable release. It does not deploy. Use the source-bound
plan author above; source plans and candidate manifests are independently pinned.

```sh
target/release/robin-highscores-manifestctl assemble-vps-release-v2 \
  vps-release-plan-v2.json \
  /home/robinhood/.local/opt/robin-highscores/releases/COMMIT.partial
target/release/robin-highscores-manifestctl validate-vps-release-v2 \
  /home/robinhood/.local/opt/robin-highscores/releases/COMMIT.partial
```

The output is the absent exact `COMMIT.partial` path. Activation promotes the
retained candidate inode to its `COMMIT` sibling with no replacement. Validation
accepts only those two names, never an arbitrary suffix or copied staging tree.

### Canonical plan and deployment identity

The input is canonical JSON for `VpsReleasePlanV2`. It contains schema version
2, the exact source commit, the complete publication-v3 root, five binaries,
four configs, nineteen reviewed host files, and two external raw-root
declarations. Every source file is an `ArtifactRefV1` pin. Typed lists must be
in enum order; unknown, duplicate, missing, reordered, zero-digest,
placeholder, noncanonical, or changed inputs fail closed.

For deployment, the plan is retained as the mode-`0400` file
`incoming/.sources-COMMIT/vps-release-plan-v2.json`, and every source path in
it is below that exact uploader closure. The outer transaction receives the
plan as an inherited descriptor plus independently reviewed plan and candidate
V2-manifest digests. Source consumption revalidates the plan, the candidate's
PublicationV3 lock, and the retained candidate inode under the one inherited
activation lock; it removes only `.sources-COMMIT`, never the candidate. A
rollback accepts no plan descriptor and has no source-consumption operation.

The V2 release manifest also binds the exact database migration level through
the shared `HIGHSCORES_DATABASE_SCHEMA_VERSION` authority. A manifest from an
older release schema, one missing this identity, or one naming a different
database schema is rejected before deployment or backup verification.

The exact binary roles are:

```text
admin
manifest_tool
server
worker
replay_verifier
```

There is no broker binary. The manifest tool is included so
`deploy/validate-release-bundle.sh` can run the typed validator on the VPS.
The exact configs are server, worker, `api.env`, and `worker.env`; both env
files contain only `RUST_LOG=info`. No broker config is accepted.

The release manifest binds this fixed user deployment identity:

```text
user                  robinhood
home                  /home/robinhood
install_root          /home/robinhood/.local/opt/robin-highscores
current_link          /home/robinhood/.local/opt/robin-highscores/current
persistent_state_root /home/robinhood/.local/share/robin-highscores
```

Accepted releases live at `install_root/releases/COMMIT`. Activation atomically
switches `current`, but services and configs are rendered with the exact
commit-named release path. They never execute or open security-sensitive input
through `current`.

### Exact bundle layout

```text
SOURCE_COMMIT
SHA256SUMS
MODE_INVENTORY
vps-release-manifest-v2.json
bin/
  robin-highscores-admin
  robin-highscores-manifestctl
  robin-highscores-server
  robin-highscores-worker
  robin-replay-verifier
config/
  highscores-server.toml
  highscores-worker.toml
  api.env
  worker.env
  manifests/...
private/
  raw-root-declarations-v2.json
  source-tree-manifests-v2/...
  verifier-bundles/...
  verifier/operator-config/...
  campaign-states/...
publication/
  backend-publication-v3.json
  publication-manifest-v3.json
  publication-manifest-v3.sha256
  publication-lock-v3.json
  publication-lock-v3.sha256
systemd/user/
  robin-highscores.target
  robin-highscores-api.service
  robin-highscores-worker.service
  robin-highscores-backup.service
  robin-highscores-backup.timer
deploy/
  DEPLOY_BOOTSTRAP_SHA256SUMS
  deploy-release.sh
  rollback-release.sh
  validate-release-bundle.sh
  tests/
    real-runtime-fence-release-gate.sh
    real-runtime-fence-e2e.py
    real-runtime-fence-e2e-selftest.py
  root-once.sh
  nginx-robinhood-api.challenge.conf
  nginx-robinhood-cloudflare-only.conf
  nginx-robinhood-api.locations.conf
  nginx-robinhood-api.vhost.conf
  README.md
  VPS_RELEASE_INSTALL.md
  BACKUP_RESTORE.md
```

`config/manifests`, official verifier bundles, the private verifier catalog,
canonical campaign templates, and official source-tree manifests are copied
unchanged from the validated publication. The embedded publication manifest
and complete lock prove the exact subset. Browser/static roots, Cloudflare
identity material, raw game installations, databases, uploaded replay or
campaign objects, backups, mutable status, keys, credentials, tokens, and SSH
material are categorically absent.

The five user units are installed below
`/home/robinhood/.config/systemd/user`. They contain no `User=`, `Group=`,
multi-user target, root-system unit path, broker, polkit, or `systemd-run`
authority. API, worker, and backup `ExecStart` values name binaries and configs
below the exact commit release. The target is enabled from `default.target`;
the backup timer activates the user backup service. `deploy/root-once.sh` and
the nginx include are reviewed, separate root-once proxy inputs. They grant no
root service-manager or principal-creation authority and are never run by
ordinary release activation.


### Direct verifier launch

The worker config has no broker socket, response timeout, root UID, polkit, or
service-manager field. Its required `[verifier_launcher]` table binds:

```text
bwrap_program             /usr/bin/bwrap
prlimit_program           /usr/bin/prlimit
verifier_program          .../releases/COMMIT/bin/robin-replay-verifier
wall_timeout_seconds      120
cpu_limit_seconds         120
address_space_limit_bytes 1073741824
process_limit             32
open_files_limit          128
file_size_limit_bytes     134217728
max_request_bytes         1048576
```

The two host programs and verifier each have a nonzero lowercase SHA-256 pin;
the verifier pin must equal the publication verifier. The verifier path, job
catalog, Demo/Full source manifests, and worker's server-config path all name
the same exact commit release. The campaign store names the persistent state
root. Resource-envelope substitutions and legacy root-level broker or verifier
fields fail validation.

### Integrity and validation

Finished releases are single-owner and read-only. Directories, binaries, and deploy scripts use
mode `0550`; other regular files use `0440`. `MODE_INVENTORY` lists the root,
every directory, and every regular file, including itself and `SHA256SUMS`.
`SHA256SUMS` is sorted by canonical relative path and covers every regular file
except itself. The typed manifest independently inventories every payload file
with its exact SHA-256, byte length, media type, and mode, and binds the source
commit, deployment identity, publication manifest, publication lock, and
verifier.

`validate-vps-release-v2` needs no source plan. It rejects omissions, unsafe
extras, substitutions, mode changes, path escapes, symlinks, hardlinks,
special nodes, mutable files, secrets/config drift, static-origin leakage,
broker/polkit/socket artifacts, root system services, publication-lock drift,
campaign-template drift, verifier substitution, raw-root drift, and a
directory/source-commit mismatch. Run it before activation and after any
transfer. The deployment bootstrap separately pins the deploy/rollback shell
and validator bytes; the outer manifest tool self-attests and keeps one
canonical activation-lock open-file description through exec. Release
deployment and rollback must stop on any validation error; they never
synthesize missing data, repair runtime authorities, use shell `du` as
capacity admission, or silently downgrade authority.

## VPS installation and rollback

### Host layout

```text
/home/robinhood/.local/opt/robin-highscores/
  activation.lock                    one OFD lock for deploy and rollback
  incoming/.sources-<commit>/        plan-bound uploader source closure
  current -> releases/<40-lowercase-hex-commit>
  releases/<commit>.partial          retained sealed candidate inode
  releases/<commit>/                 immutable release, dirs 0550/files 0440
/home/robinhood/.config/systemd/user/ user target/services/timer
/home/robinhood/.local/share/robin-highscores/
  database/                           persistent mutable state
  replays/
  campaign-states/
  backups/
  status/                            API-readable authenticated backup status only
  api-secrets/                       five VPS-generated owner-only secrets
  runtime-fence/                     immutable database lock-file authority
  raw-content/demo/                   manual, unbundled, read-only licensed data
  raw-content/full/
```

The exact commit-named release is the runtime authority. Final server/worker
configuration and user units pin that absolute path; the worker also pins the
exact verifier, `/usr/bin/bwrap`, and `/usr/bin/prlimit` digests. `current` is
an atomic operational selector, not an escape from those pins.



This is the production procedure for `/home/robinhood`. Substitute no other
user, home, release root, or state root: the typed bundle and runtime configs
pin these paths deliberately.

### 1. One root operation

Assemble a directory containing exactly the reviewed root-once files:

```text
ROOT_ONCE_SHA256SUMS
root-once.sh
nginx-robinhood-api.challenge.conf
nginx-robinhood-api.locations.conf
nginx-robinhood-api.vhost.conf
nginx-robinhood-cloudflare-only.conf
```

Before invoking root, independently compare both `root-once.sh` and
`ROOT_ONCE_SHA256SUMS` with the published hashes. Copy the script to a
root-owned temporary path, then run one command with the absolute kit path and
out-of-band manifest digest:

```sh
sh /root/reviewed-root-once.sh \
  /home/robinhood/robin-highscores-root-once \
  EXPECTED_64_LOWERCASE_HEX_ROOT_ONCE_MANIFEST_SHA256
```

The script first copies the manifest and nginx sources into a private
root-owned directory and verifies copied bytes. On a host without a certificate
it installs the HTTP-only ACME/404 vhost, writes an unpredictable challenge,
and requires the public Cloudflare path to return its exact bytes. It refuses
to register an account: an existing Certbot account must already be present.
It obtains/renews the certificate with `certonly --webroot`, installs the final
port-80 ACME and port-443 Cloudflare-only vhost, runs `nginx -t`, and reloads.
It then probes that TLS vhost directly over loopback with the production SNI
name and requires the private nginx origin marker. The API upstream is
deliberately still absent and may return 502; the earlier public HTTP-01 probe
already proved the Cloudflare route to the origin.

Rerunning is idempotent. A failure restores the prior managed vhost/includes
and reloads the last valid nginx configuration. This step also runs
`loginctl enable-linger robinhood`. It grants no sudo, polkit, system service,
or nginx access to the deploy user.

Review and update the pinned Cloudflare ranges when Cloudflare changes its
published lists. Keep the host firewall's port 80/443 allowlist synchronized.

### 2. Provision clean-first authorities and licensed roots

Create these owner-only regular files on the VPS, never in an upload or bundle:

```text
/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key
/home/robinhood/.local/share/robin-highscores/api-secrets/competition-run-grant.key
/home/robinhood/.local/share/robin-highscores/api-secrets/run-preflight-grant.key
/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token
```

The first three are exactly 32-byte binary values generated by the reviewed
admin commands. Generate them before final authority publication, expose only
the two printed Ed25519 public keys to the publication workflow, and keep all
secret bytes on the VPS/off-host secret backup. The final competition and
ruleset manifests must pin those public keys. Create the printable 32..128-byte
moderation token without displaying it. All four files must be owned by
`robinhood`, mode `0400`, link count one, and have no symlink component.
Each initialization command uses a minimal bootstrap read of only its named
absolute path from the supplied TOML. It does not require final manifests, the
database, or either other secret; normal serving and migration commands still
load and validate the complete production configuration.

Provisioning order is strict: author a private bootstrap TOML containing the
three absolute key paths, run `initialize-cursor-key`,
`initialize-competition-run-grant-key`, and
`initialize-run-preflight-grant-key` with the reviewed admin binary, then use
the two printed public keys to finish the publication manifests. Separately
generate and install the moderation token before assembling the final server
config or attempting migration, deployment, or the first backup. The
moderation token has no public-key initialization command and must never be
printed. Only after all four private files exist should the final authority and
release bundle be validated.

Do **not** manually create either of these clean-first authorities:

```text
/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key
/home/robinhood/.local/share/robin-highscores/runtime-fence
```

The first deployment first proves the exact four-key/fence-absent state, makes
an initialization journal durable, creates the 32-byte mode-`0400` fifth key,
and publishes one mode-`0500` fence containing only empty mode-`0400`
`db-admission.lock` and `db-quiescence.lock`. It then proves the exact
five-key/fence-present state before continuing. A crash may be resumed only
through that exact journal. Upgrades and rollbacks adopt the same key and fence
inodes and fail rather than chmod, replace, recreate, or otherwise repair them.

Manually install licensed Demo and Full trees at:

```text
/home/robinhood/.local/share/robin-highscores/raw-content/demo
/home/robinhood/.local/share/robin-highscores/raw-content/full
```

These roots never appear in a bundle, backup, site, or API response. After
copying, require owner `robinhood`, directories mode `0550`, singleton regular
files mode `0440`, and nonempty trees without symlinks, special nodes, hard
links or mount crossings. The worker
hashes the complete trees against the exact release's selected V2 manifests
before leasing work. Directory existence alone is not acceptance evidence.

### 3. Review and deploy a release

Upload or assemble the sealed candidate at exactly
`~/.local/opt/robin-highscores/releases/<commit>.partial`. Retain its complete
uploader source closure at exactly `incoming/.sources-<commit>` and its
canonical mode-`0400` plan at
`.sources-<commit>/vps-release-plan-v2.json`. The plan must name only sources
inside that exact closure and must bind its PublicationV3 authority.

Obtain all of the following through a separate trusted review channel:

- the 40-hex commit;
- the candidate `SHA256SUMS` SHA-256;
- the three-entry `DEPLOY_BOOTSTRAP_SHA256SUMS` SHA-256;
- the canonical plan SHA-256;
- the candidate `vps-release-manifest-v2.json` SHA-256; and
- the exact reviewed manifest-tool executable used to enter the transaction,
  byte-identical to the candidate's manifest-tool role.

Prepare a private bootstrap directory outside the managed install/state roots.
It contains exact bytes for `deploy-release.sh`, `rollback-release.sh`,
`validate-release-bundle.sh`, and `DEPLOY_BOOTSTRAP_SHA256SUMS`. The two
scripts used as inherited descriptors are mode `0500`; the checksum manifest
is mode `0400`; every file is owner `robinhood`, link count one, and reached
without a symlink. The manifest tool is mode `0550`. Do not invoke the inner
deployment shell directly: it accepts only inherited authority from the outer
Rust transaction wrapper.

As `robinhood`, run exactly one release operation by opening distinct
descriptors and executing the reviewed manifest tool once. This illustrative
shell uses the frozen argument order; substitute only the reviewed absolute
paths and out-of-band values:

```sh
candidate=/home/robinhood/.local/opt/robin-highscores/releases/COMMIT.partial
plan=/home/robinhood/.local/opt/robin-highscores/incoming/.sources-COMMIT/vps-release-plan-v2.json
bootstrap=/home/robinhood/robin-highscores-deploy-bootstrap
manifestctl=/absolute/reviewed/robin-highscores-manifestctl

exec 3<"$bootstrap/deploy-release.sh"
exec 4<"$bootstrap/DEPLOY_BOOTSTRAP_SHA256SUMS"
exec 5<"$bootstrap/validate-release-bundle.sh"
exec 6<"$manifestctl"
exec 7<"$plan"
exec /proc/self/fd/6 exec-vps-activation-v2 deploy \
  /proc/self/fd/3 /proc/self/fd/4 /proc/self/fd/5 /proc/self/fd/6 \
  /proc/self/fd/7 EXPECTED_PLAN_SHA256 EXPECTED_VPS_MANIFEST_SHA256 -- \
  "$candidate" COMMIT EXPECTED_SHA256SUMS_SHA256 EXPECTED_BOOTSTRAP_SHA256SUMS_SHA256
```

The outer wrapper authenticates every descriptor and digest before acquiring
the single canonical `activation.lock`, then retains that same lock open-file
description across the complete shell transaction. The candidate directory is
pinned before the lock and remains the exact same inode while Rust performs a
no-replace sibling rename from `releases/COMMIT.partial` to `releases/COMMIT`; there is
no private release copy or staging tree. Source consumption validates the
retained plan, candidate V2 manifest, and PublicationV3 lock and removes only
the uploader `.sources-COMMIT` closure. It never removes the candidate.

Before that promotion or any database, selector, unit, service, or
clean-first-authority mutation, candidate deployment must pass the bundled
`deploy/tests/real-runtime-fence-release-gate.sh`. It cannot be disabled. The
gate receives the retained candidate descriptor, authenticates both licensed
raw trees recursively, masks all production mutable state, and runs the exact
candidate server, worker, and BackupV4 admin in a disposable private
`bwrap` PID/network namespace. Missing or changed gate files, descriptor
substitution, a failed self-test, or a failed real-process proof aborts the
transaction without mutation. Do not invoke the gate manually as a substitute
for the outer activation command.

On an upgrade, the transaction disables and drains the timer, stops and proves
all writers inactive, takes an offline source BackupV4 generation, and admits
it with `verify-transaction-backup`. It verifies the live migration history
through the candidate descriptor and permits only a supported forward
migration to the candidate's compiled current schema. First deploy initializes
that current schema. Shell byte totals are not capacity authority:
`estimate-backup-space --require-available` gates each backup boundary.

After migration or same-schema selection, all target writers remain stopped
while a distinct target BackupV4 generation is produced and admitted as a
canonical `BackupVerificationReceiptV2`. Only then may the API start and report
ready, the worker start, and the target and timer become enabled. The API can
read authenticated `status/backup-status.json` but cannot traverse `backups/`;
the worker can access neither. All three runtime services receive the immutable
runtime fence read-only. Backup payloads exclude the fifth backup authority,
runtime fence, immutable release, static site, datadirs, raw roots, and
symlink-bearing systemd user directory.

For a crash after exact-inode promotion, use the same outer command with
`--resume-installed` as the first business argument and the exact retained
`releases/COMMIT` path as the candidate. Keep every out-of-band digest and the
plan/source evidence unchanged. Do not synthesize a new plan, copy a release,
or bypass the outer lock.

Useful inspection commands do not require root:

```sh
systemctl --user status robin-highscores.target
systemctl --user status robin-highscores-api.service
systemctl --user status robin-highscores-worker.service
systemctl --user status robin-highscores-backup.timer
journalctl --user -u robin-highscores-api.service --since today
journalctl --user -u robin-highscores-worker.service --since today
curl --fail --silent --show-error http://127.0.0.1:8787/healthz
curl --fail --silent --show-error http://127.0.0.1:8787/readyz
```

### 4. Rollback

Rollback has no plan descriptor and no source-consumption authority. Obtain
the target release's original `SHA256SUMS` digest and its three-entry bootstrap
digest through the trusted channel, prepare the same sealed bootstrap
descriptors, use a reviewed manifest tool byte-identical to the target release's
manifest-tool role, and enter the disjoint outer operation:

```sh
bootstrap=/home/robinhood/robin-highscores-deploy-bootstrap
manifestctl=/absolute/reviewed/robin-highscores-manifestctl

exec 3<"$bootstrap/rollback-release.sh"
exec 4<"$bootstrap/DEPLOY_BOOTSTRAP_SHA256SUMS"
exec 5<"$bootstrap/validate-release-bundle.sh"
exec 6<"$manifestctl"
exec /proc/self/fd/6 exec-vps-activation-v2 rollback \
  /proc/self/fd/3 /proc/self/fd/4 /proc/self/fd/5 /proc/self/fd/6 -- \
  TARGET_COMMIT EXPECTED_TARGET_SHA256SUMS_SHA256 EXPECTED_BOOTSTRAP_SHA256SUMS_SHA256
```

Before changing the timer, services, units, selector, database, or status,
rollback authenticates the current release, target release, runtime authority,
and live database and requires one identical schema. Any mismatch is a
zero-mutation refusal. It transaction-verifies distinct source and target
BackupV4 generations, installs the target's exact units, atomically selects it,
then repeats readiness and worker checks. It never migrates, restores, repairs,
or consumes uploader sources. A failed post-selection rollback leaves the
target stopped and reports that state instead of returning fake success. Resume
uses `--resume-target` as the first business argument and the same pinned
digests; it is not a separate compatibility lane.

## Backup and disaster recovery

The daily `robin-highscores-backup.timer` runs in the lingering `robinhood`
user manager. Its oneshot uses the exact active commit's admin binary and
configuration, retains two complete local generations below
`~/.local/share/robin-highscores/backups`, and publishes
`status/backup-status.json` only after the new generation passes complete
offline verification.

Three distinct canonical documents make up the backup evidence:

- `BackupManifestV4` is the protected payload manifest in
  `backup-manifest.json`. It contains the complete release-bound inventory,
  hashes, database schema, restore mappings, file modes, and byte counts. It
  may be large and is admitted independently under the manifest byte limit.
- `BackupVerificationEnvelopeV2` is a compact HMAC-authenticated document in
  each completed backup directory. It binds the exact manifest digest,
  release identity, database schema, counts, result, and backup ID. This is the
  durable proof used to authenticate a historical generation after it is no
  longer the latest backup.
- `BackupStatusV4` is the compact HMAC-authenticated latest-backup summary at
  `status/backup-status.json`. It contains the backup ID/path, manifest digest,
  release identity, database schema, counts, bytes, and publication time. It
  does **not** embed `BackupManifestV4`. One temporary-file rename followed by
  a status-directory fsync is the latest-status publication boundary.

The API sandbox may read the owner-only compact status document but cannot
access backup payloads; the worker can access neither. `/readyz` authenticates
the summary, checks its release/schema identity and 32-hour freshness, and
deliberately does not open or continuously reverify the protected generation.
Missing, stale, future-dated, malformed, unauthenticated, or wrong-release
status keeps `/readyz` unavailable while `/healthz` can remain live. Deletion
or substitution of a protected payload is detected by the next backup,
retention verification, or explicit restore verification rather than by each
readiness request.

### Backup authority and recovery custody

Backup status and per-generation verification envelopes use a dedicated fifth
secret:

```text
/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key
```

It is separate from the cursor HMAC key, competition grant seed,
run-preflight grant seed, and moderation credential. The loader requires an
effective-user-owned, exact 32-byte, nonzero regular file with mode `0400`, one
hard link, no symlink traversal, and a private mode-`0700` parent. On a clean
host, only the activation transaction may invoke
`initialize-backup-authority-key`: it first proves the exact absent state and
makes its runtime-authority initialization journal durable. The command uses
create-new semantics, fails if the path already exists, and never prints key
bytes. A post-crash retry must be bound to that exact journal. Upgrade and
rollback require the key to be present and exact before mutation and never
regenerate or repair it.

The backup-authority key is intentionally **not archived** in
`BackupManifestV4`, a restore-source map, or any backup payload. Preserve it in
separately controlled disaster-recovery custody. A historical backup requires
the exact authority key that authenticated its
`BackupVerificationEnvelopeV2`; the payload alone is insufficient authority.
Errors, receipts, logs, release bundles, and public artifacts must never
contain the key bytes.

Every completed generation also depends on an independent, append-only copy of
its exact `VpsReleaseManifestV2` bytes. The backup authority stores that file at
`backups/.release-authorities-v2/<vps-release-manifest-sha256>.vps-release-manifest-v2.json`;
the store is mode `0700` and each indexed regular file is mode `0400`. Export
the referenced indexed file with the generation, without renaming or changing
its bytes or metadata. Historical verification requires both the valid HMAC
envelope and this separately indexed release authority. Neither is a
self-asserting substitute for the other.

There is currently one authority-key path and no key identifier or keyring, so
in-place automatic rotation is forbidden. The old key must remain available
until every backup and envelope authenticated by it has been pruned from the
live retention set. Before an operator-driven rotation, verify and export all
old generations with the old key, preserve that key with their off-host DR
records, remove the old generations from the live set under a reviewed
maintenance procedure, install a newly generated authority, and immediately
create and export a fresh verified generation. Automatic deploy and rollback
must preserve the existing authority unchanged.

The runtime fence described in installation is a separate, non-backup
recovery authority. Automation must preserve its exact inodes and fail on drift.

The backup payload contains mutable SQLite, replay objects, campaign objects,
the four ordinary durable API credentials, and the five installed regular
user-unit files. It never contains the fifth backup authority, an immutable
release, release configuration/manifests, the static site, game datadir, raw
Demo/Full roots, or the symlink-bearing systemd user directory.
`BackupManifestV4`, `BackupVerificationEnvelopeV2`, and `BackupStatusV4` bind
the exact source commit, database schema, canonical installed
`vps-release-manifest-v2.json` digest, and publication-lock digest. Immutable
release bundles, licensed raw roots, the backup authority, and runtime-fence
authority need separate protected recovery sources. Nothing below `backups/`
belongs in a release bundle, web closure, log, or public upload.

### Routine checks

```sh
systemctl --user start robin-highscores-backup.service
systemctl --user status robin-highscores-backup.service
journalctl --user -u robin-highscores-backup.service --since today
systemctl --user list-timers robin-highscores-backup.timer
curl --fail --silent --show-error http://127.0.0.1:8787/readyz
```

The canonical `backup-and-publish-status` workflow acquires the durable backup
gate before new writers can enter, drains all bounded writer classes, keeps the
gate alive, takes SQLite's online snapshot, scrubs snapshot-only transient
leases, checks every content-addressed object, fsyncs files/directories,
verifies the protected `BackupManifestV4`, publishes the per-backup
`BackupVerificationEnvelopeV2`, and only then atomically publishes compact
`BackupStatusV4`. Every normal, error, timeout, or cancellation path releases
its gates. Existing database operations drain before the snapshot; database-
backed requests may wait briefly while the snapshot owns the database fence.
The genuinely non-database `/healthz` check remains available. Never synthesize
or copy any of the three evidence documents by hand.

Before estimating capacity, the workflow removes only bounded, owner- and
filesystem-matched `.backup-v4-*.partial` directories left by an interrupted
job. A malformed name, symlink, hard link, foreign owner/device, special node,
or excessive topology fails closed instead of being followed or removed.

An activation transaction verifies the backup it just created with
`verify-transaction-backup`. The caller passes pinned descriptors for the
canonical `backups/` root, compact current status, dedicated backup-authority
key, and exact `VpsReleaseManifestV2` authority. Rust authenticates the compact
status, requires its bounded backup ID/path to select exactly
`backups/<backup-id>`, opens that child capability-relatively without links or
mount crossing, verifies its per-generation envelope and full protected
manifest/payload, and emits a canonical `BackupVerificationReceiptV2`.
Transaction receipts include `current_status` evidence binding the exact
compact status bytes. No shell JSON parsing, embedded status manifest, archived
cursor key, or directory guessing participates in admission.

`verify-backup` is the independent historical/offline mode. It does not take
or trust the latest status document and continues to require the independently
recorded `--expected-backup-manifest-sha256`. It authenticates the selected
generation's `BackupVerificationEnvelopeV2` with the separately preserved
backup-authority key and requires the exact preserved `VpsReleaseManifestV2`
authority. Its receipt has no `current_status` evidence. The transaction and
historical CLI contracts cannot be mixed.

The configured 1 GiB free-space readiness floor is not backup capacity. The
typed `estimate-backup-space --require-available` command and the backup itself
compute the same `BackupSpaceEstimateV1` immediately before copying. The bound
rounds every copied regular file up to the destination filesystem allocation
granularity, charges prospective directories/entries, checks inode
availability when reported, and independently reserves the bounded manifest,
per-backup verification envelope, compact atomic-status temporary, and SQLite
DB/WAL/SHM plus concurrent-writer margin. Its canonical newline-free JSON binds
the exact backup root, status path, source release/unit identity, and
restore-source-map closure. Shell `du --bytes` output is not authorization.

Publication happens before pruning and retains two complete backups. Existing
backups and releases already reduce live available space, so the typed
requirement adds exactly one new backup scratch generation plus the configured
1 GiB floor. A 5 GiB VPS is sufficient only while the live check succeeds at
every source and target backup boundary. Monitor database/WAL growth, both
object stores, verifier output, retained backups, inode availability, and
filesystem headroom. A local backup is not disaster recovery: export every
completed generation, its exact release authority, recorded manifest digest,
and the corresponding externally held backup-authority key to separately
controlled storage without changing bytes.

### Offline historical restore drill

Restore only onto an empty isolated host with Cloudflare/nginx admission
disabled and the API, worker, target, backup service, and timer absent or
stopped:

1. Select the exact `backup-v4-<...>` generation from protected/off-host
   custody and obtain its independently recorded `BackupManifestV4` digest.
   Do not use latest `backup-status.json` to select or authorize a historical
   generation; latest status may be missing, stale, or legitimately point to a
   newer backup.
2. Retrieve the separately preserved backup-authority key that signed this
   generation, the immutable release bundle for exactly its source commit, and
   the exact digest-indexed authority file exported from
   `.release-authorities-v2`.
   Validate the bundle and require its canonical
   `vps-release-manifest-v2.json`, database schema, publication-lock digest,
   and source commit to equal the selected backup evidence. Never substitute
   `current`, a newer release, a same-basename file, a newly generated HMAC
   key, or an unvalidated admin binary.
3. Reconstruct the canonical owner-only backup root and its
   `.release-authorities-v2` child. Install the exact indexed authority at
   `<vps-release-manifest-sha256>.vps-release-manifest-v2.json` with directory
   mode `0700` and file mode `0400`; do not pass an arbitrary same-basename
   release file. Pin the canonical backup root, selected backup child, and
   exact backup-authority key as inherited descriptors. Open them with
   no-follow semantics, keep the descriptors open across exec, and use the
   preserved release's admin binary to run `verify-backup` with
   `--backup-root-fd`, `--backup-directory-fd`, and
   `--backup-authority-key-fd`. Also pass the independently recorded
   `--expected-backup-manifest-sha256`, `--expected-source-commit`,
   `--expected-vps-release-manifest-sha256`, and
   `--expected-publication-lock-sha256`. Historical verification deliberately
   has no `--status-envelope-fd`.

   The verifier authenticates `backup-verification-envelope.json`, requires it
   to match the protected `BackupManifestV4`, verifies every listed byte and
   SQLite invariant, and rejects missing/unlisted bytes, wrong schemas,
   substituted descriptors, links, mount crossings, release/static/datadir
   payloads, and non-allowlisted paths. It loads no server config, installed
   current release, latest status, or secret outside the pinned descriptors.

   On exit status zero, stdout is exactly one canonical newline-free
   `BackupVerificationReceiptV2` binding the verification-envelope digest and
   length, selected directory/ID, manifest digest, release identity, database
   schema, file/directory counts, and bytes. Its `current_status` is `null`.
   Canonical-parse and durably persist those exact bytes before restoring.
   Treat nonzero exit, empty/additional stdout, noncanonical JSON, or any field
   mismatch as no receipt.
4. Restore only SQLite, replay objects, campaign objects, and the four archived
   ordinary API credentials to an empty staging hierarchy using the manifest's
   absolute mappings. Set each credential to owner-only mode `0400`. Install
   the externally preserved backup-authority key separately at its exact path;
   do not extract or regenerate it from the backup. Never extract blindly over
   `/home/robinhood`; reject symlinks, hard links, special files, unexpected
   owners/modes, or unlisted bytes.
5. Compare the five archived unit bytes with the five units in the validated
   release. They are recovery evidence, not activation authority: do not copy,
   enable, or start archived units. Restore licensed Demo/Full roots from their
   separate source and revalidate them against the exact release.
6. Activate the exact restored release first. The
   restored database schema must already equal that release's authenticated
   schema. Create and transaction-verify a target-release backup before
   `/readyz` and before starting the worker. If a newer schema is desired,
   first complete and review this exact-source recovery checkpoint, then use
   the normal authenticated forward-deploy path: validate the append-only
   migration prefix, take another source checkpoint, stop all services,
   migrate forward, and create/verify the new target backup before restart.
7. Re-enable the Cloudflare route only after the nginx trust boundary, signer
   origin, static closure, raw-root inventories, target backup, timer, and
   off-host copy all pass review.

Record backup ID/manifest digest, verification-envelope and receipt digests,
source and target release identities, database schema, which externally held
authority applies, restore duration, verification output, and reviewer
identity outside the host. Drill after schema, secret, path, backup, verifier,
or unit-contract changes.

The 32-hour readiness age is derived from the daily interval, up to 45 minutes
of timer jitter, the six-hour service timeout, and at least one hour of margin.
A 26-hour setting is unsafe and rejected by production bundle validation.

## Cloudflare deployment and rollback

This path deploys the accepted leaderboard site without GitHub repository or
Environment administration. It does not weaken the immutable release
contracts. A normal release contains only the public site, isolated signer,
and versioned `/wasm` corpus. Licensed datadir bytes are a separate, rare,
manual artifact and are never copied into the normal deployment bundle.

### Required token and origin

Create a **user-owned** token under **My Profile > API Tokens**. Account-owned
tokens are rejected by the legacy Page Rules audit (Cloudflare code 1011).
Restrict the token to account `phiresky` and zone `phiresky.xyz`:

- Account: `Workers Scripts: Edit`, `Account Settings: Read`.
- Zone `phiresky.xyz`: `Zone: Read`, `Workers Routes: Edit`, `DNS: Read`,
  `Cache Rules: Read`, `Page Rules: Read`.

No DNS, Cache Rule, Page Rule, KV, R2, Pages, or Tail write permission is
needed. Put these names in the ignored root `.env`:

```text
CLOUDFLARE_ACCOUNT_ID=8bbf14bddc9c93d4ec06a53a055b7bb0
CLOUDFLARE_ZONE_ID=f31bf0d0dad316673c06fccdd641ad85
CLOUDFLARE_API_TOKEN=...
```

Before deployment, the proxied `A` record
`robinhood.phiresky.xyz -> 65.109.224.29` must exist and the VPS API must be
healthy. Do not create `identity.robinhood.phiresky.xyz`; the signer Custom
Domain creates and manages that record and certificate.

Use the release-authorized Node.js 24.19.0, pnpm 12.3.4, Wrangler 4.127.1, and
the `robin-highscores-manifestctl` built from the exact release commit. The
scripts reject a different Node, source commit/tree, `Cargo.lock`, approved
materialization receipt, PublicationV3 manifest/lock, or physical artifact
inventory.

### Standalone Demo datadir authority

Datadir bytes are deployed only through their dedicated Worker and never enter
a runtime, public-site, publication, or VPS bundle. Assemble and authorize the
canonical Demo converter output locally:

```sh
node wasm-www/scripts/assemble-datadir-corpus.mjs \
  --initial /path/to/demo-web-shipping datadir-dist
node wasm-www/scripts/datadir-release-authority.mjs author \
  datadir-dist SOURCE_COMMIT CARGO_LOCK_SHA256 \
  datadir-inventory.json datadir-authority.json
node wasm-www/scripts/datadir-release-authority.mjs verify \
  datadir-dist datadir-inventory.json datadir-authority.json \
  EXPECTED_AUTHORITY_SHA256
```

For an immutable re-publication use `--update PREVIOUS_DATADIR_DIST
DEMO_CONVERTER_OUTPUT DATADIR_DIST`; changed Demo bytes fail. The protected manual
`deploy-static-datadir.yml` alternative deploys `robinhood-datadir-assets` without
attaching routes, proves the exact current Worker version, and then emits
canonical `datadir-deployment.json`.

The authority's `source_commit` and `cargo_lock_sha256` are provenance for the
producer of this independently deployed corpus. They remain internally bound
to its inventory and deployment receipt, but are not required to equal the
source or lock of a later runtime/publication that reuses the same immutable
datadir. Manual datadir deployment still runs from the authority's exact
producer checkout.

### Complete wasm runtime authority

`build-static-runtime.yml` produces a non-deployable immutable **addition**. It
deliberately omits datadir bytes, deployment receipts, and `_headers`. After
the standalone datadir deployment above, assemble a wasm-only runtime that
binds its authority and post-deploy receipt:

```sh
node wasm-www/scripts/assemble-runtime-corpus.mjs \
  --initial runtime-addition datadir-authority.json \
  datadir-deployment.json runtime-dist
```

For an update, retain every already published immutable build and replace only
the mutable `wasm/latest.json` pointer in a new output directory:

```sh
node wasm-www/scripts/assemble-runtime-corpus.mjs \
  --update previous-runtime-dist runtime-addition \
  datadir-authority.json datadir-deployment.json runtime-dist
```

The assembler copies only the canonical receipt to
`/wasm/datadir-deployment.json`. Every build's Demo URL, length, object digest,
and native source identity must match it. Complete runtime verification also
requires the exact external `datadir-authority.json`; omission or substitution
fails closed. No `/datadirs` path is accepted in the runtime corpus.

Generate and verify the exact physical asset inventory outside the corpus:

```sh
node wasm-www/scripts/write-static-origin-inventory.mjs \
  runtime runtime-dist SOURCE_COMMIT CARGO_LOCK_SHA256 runtime-authority.json
node wasm-www/scripts/verify-static-origin-inventory.mjs \
  runtime runtime-dist runtime-authority.json EXPECTED_INVENTORY_SHA256
```

The approved Actions artifact consumed by deployment must have exactly this
layout, with no enclosing release directory:

```text
runtime-dist/
runtime-authority.json
datadir-authority.json
```

Preserve the complete corpus, its exact inventory bytes, the inventory SHA-256,
and its Actions run/artifact identities in durable operator release storage.
Actions retention is a transfer/evidence convenience, not the long-term release
archive. Never upload an entire operator publication or private release tree.


### One-time or rare datadir deployment

Build `datadir-dist/`, `datadir-inventory.json`, and
`datadir-authority.json` with the checked-in datadir assembler and authority
tooling. The preflight below verifies the corpus and independently reproduces
the physical inventory. Its stage path must not exist:

```sh
node wasm-www/scripts/deploy-datadir-operator.mjs \
  --datadir /absolute/artifacts/datadir-dist \
  --inventory /absolute/artifacts/datadir-inventory.json \
  --authority /absolute/artifacts/datadir-authority.json \
  --authority-sha256 APPROVED_AUTHORITY_SHA256 \
  --stage /absolute/staging/datadir-preflight
```

After reviewing the dry run, repeat with fresh absent stage, receipt, and
private evidence paths:

```sh
node wasm-www/scripts/deploy-datadir-operator.mjs \
  --datadir /absolute/artifacts/datadir-dist \
  --inventory /absolute/artifacts/datadir-inventory.json \
  --authority /absolute/artifacts/datadir-authority.json \
  --authority-sha256 APPROVED_AUTHORITY_SHA256 \
  --stage /absolute/staging/datadir-live \
  --receipt /absolute/artifacts/datadir-deployment.json \
  --evidence /absolute/private-evidence/datadir-DEPLOYMENT_ID \
  --execute
```

This uploads `robinhood-datadir-assets` without attaching or changing a route,
proves its exact current 100% version, and emits the canonical deployment
receipt. An update corpus must preserve every previously published immutable
path. The receipt is an input to the runtime assembler; normal release tooling
never reads `datadir-dist/`.

### Normal release bundle

Use the PublicationV3 materialization and six-argument OperatorBundleV2 assembly
commands in [Release authoring](#release-authoring). The independently approved
WASM/static handoff contains exactly `runtime/` and `inventories/runtime.json`,
with directories `0550` and singleton files `0440`. JS receives the materialized
safe closure, never a Publication path or validator executable. It revalidates
all origin inventories, file metadata, source/Cargo identities and the exact
embedded datadir receipt without reminting their authorities. Record the printed
`deployment-v2.json` digest independently.

### Cloudflare Cache Rules release gate

Before every production publication, inspect the zone's enabled Cache Rules
and legacy Page Rules in addition to the Worker route list:

- reject the release if any `Cache Everything`, cache-eligibility, edge-TTL,
  or broad hostname rule can match `robinhood.phiresky.xyz/api*`;
- require every broad static caching expression to exclude the complete
  `/api` prefix, including `/api`, `/api?query`, and `/api/...`;
- run `pnpm smoke:cloudflare` after the route and cache-rule changes. Its two
  sequential metadata probes must remain `Cache-Control: no-store`, without
  `Age`, a Cloudflare `HIT`, or CORS headers; its attacker-origin preflight
  must remain the API's measured `405` without CORS; and its published
  digest-addressed ruleset probe must retain the exact one-year immutable
  policy and `nosniff`.

Record the review as private release evidence: UTC time, zone and active rule
set version, enabled rule IDs/expressions/actions in evaluation order, the
Worker route export, and the complete response-header transcript from the
smoke command. Hash the evidence bundle and retain it beside the operator's
release audit; do not copy account tokens, rule exports, or transcripts into
any public static closure.


### Zone audit and deployment

Capture a private read-only draft, inspect every DNS record, Worker route,
Cache Ruleset, and active Page Rule, then explicitly approve that exact
snapshot. The current migration retires only the legacy `/api/*` route:

```sh
node wasm-www/scripts/cloudflare-zone-audit.mjs capture \
  /absolute/private-evidence/zone-audit-draft.json

node wasm-www/scripts/cloudflare-zone-audit.mjs approve \
  /absolute/private-evidence/zone-audit-draft.json \
  /absolute/private-evidence/zone-audit-approved.json \
  OPERATOR_NAME \
  65.109.224.29 \
  'robinhood.phiresky.xyz/api/*'
```

The approval command prints the audit SHA-256. Run the complete nonmutating
preflight with fresh absent staging:

```sh
node wasm-www/scripts/deploy-cloudflare-operator.mjs \
  --bundle /absolute/cloudflare-deployment-bundle \
  --manifest-sha256 APPROVED_DEPLOYMENT_MANIFEST_SHA256 \
  --stage /absolute/staging/cloudflare-preflight \
  --zone-audit /absolute/private-evidence/zone-audit-approved.json \
  --zone-audit-sha256 APPROVED_ZONE_AUDIT_SHA256
```

This revalidates the bundle, stages only its read-only contents, dry-runs all
three exact tracked Wrangler configurations under Node.js 24.19.0, compares
live Cloudflare state byte-for-byte with the approved audit, probes the VPS API twice for uncached `no-store`
responses and a hostile preflight rejection, and proves the exact installed
datadir Worker version. It performs no Cloudflare mutation and retains no
local scratch state.

After it passes, run once with a new absent stage and evidence directory:

```sh
node wasm-www/scripts/deploy-cloudflare-operator.mjs \
  --bundle /absolute/cloudflare-deployment-bundle \
  --manifest-sha256 APPROVED_DEPLOYMENT_MANIFEST_SHA256 \
  --stage /absolute/staging/cloudflare-live \
  --zone-audit /absolute/private-evidence/zone-audit-approved.json \
  --zone-audit-sha256 APPROVED_ZONE_AUDIT_SHA256 \
  --evidence /absolute/private-evidence/cloudflare-DEPLOYMENT_ID \
  --execute
```

The live order is runtime, signer, public, then routes. Route reconciliation
creates the query-safe `/api*` no-script route first, retains the permanent
narrow `/.well-known/acme-challenge/*` nginx bypass for HTTP-01 renewal,
establishes `/wasm/*` and `/datadirs/*`, changes the broad route last, and only
then removes the explicitly approved legacy `/api/*` route. It runs the full live smoke suite
and stores response headers, new version IDs, previous deployment IDs,
digest-bound inventory authority, audit identity, and rollback identities in the mode-`0700`
private evidence directory. Each Wrangler invocation receives a fresh private
snapshot: the approved config and selected origin are mode-`0500`/`0400` in a
sealed subtree, while Wrangler's `.wrangler`, log, HOME, and XDG writes are
confined to a distinct mode-`0700` scratch directory. The sealed authority is
validated directly against the staged manifest and exact tracked config hash
immediately before and after all three dry runs and all three live uploads;
scratch is then removed through the retained directory capability.

The operating account is the trust boundary. Read-only Unix modes prevent
accidental writes and writes by other UIDs; they cannot stop a concurrently
malicious process running as the same UID from calling `chmod`. Do not run
unreviewed same-UID processes during a release. The pre/post identity proofs
detect ordinary replacement races but are not presented as cryptographic
immutability against the trusted operator UID.

If any gate fails, stop. Do not manually add a broad Worker route or reuse a
staging directory. Preserve the two evidence SHA-256 values printed by the
successful live command independently from the mode-`0700` evidence directory.

### Evidence-authorized rollback

Rollback consumes the unchanged accepted OperatorDeploymentBundleV2 and the
private evidence from its completed deployment. It does not rebuild, upload,
or infer old static bytes. Cloudflare does not reactivate an old deployment
object. Its documented rollback operation creates a new 100%-traffic
deployment of a previously published Worker version. The operator therefore
authenticates each recorded previous deployment ID and its one exact version,
then records the new active deployment ID in its transaction evidence. It does
not use the API's `force` escape hatch. See Cloudflare's
[rollback semantics](https://developers.cloudflare.com/workers/versions-and-deployments/rollbacks/)
and [Create Deployment API](https://developers.cloudflare.com/api/resources/workers/subresources/scripts/subresources/deployments/methods/create/).

Use exact Node.js 24.19.0 and first run the read-only preflight. The deployment
and rollback SHA-256 values are the independently retained values printed by
the corresponding successful deployment:

```sh
node wasm-www/scripts/rollback-cloudflare-operator.mjs \
  --bundle /absolute/cloudflare-deployment-bundle \
  --manifest-sha256 APPROVED_DEPLOYMENT_MANIFEST_SHA256 \
  --evidence /absolute/private-evidence/cloudflare-DEPLOYMENT_ID \
  --deployment-sha256 APPROVED_DEPLOYMENT_EVIDENCE_SHA256 \
  --rollback-sha256 APPROVED_ROLLBACK_EVIDENCE_SHA256
```

Preflight verifies all of the following without a Cloudflare write or local
transaction creation:

- canonical singly-linked mode-`0600` evidence beneath a pinned mode-`0700`
  directory;
- the exact OperatorDeploymentBundleV2 manifest, source, materialization,
  publication lock, datadir identity, account, zone, hostname, four release
  script names, and every additional legacy script named by a prior route;
- every recorded previous deployment and version through the Cloudflare API;
- the exact current post-deployment Worker and route state, or, when an
  existing transaction is supplied, one authenticated crash-resume prefix.

After reviewing the preflight, execute with a new absent transaction directory:

```sh
node wasm-www/scripts/rollback-cloudflare-operator.mjs \
  --bundle /absolute/cloudflare-deployment-bundle \
  --manifest-sha256 APPROVED_DEPLOYMENT_MANIFEST_SHA256 \
  --evidence /absolute/private-evidence/cloudflare-DEPLOYMENT_ID \
  --deployment-sha256 APPROVED_DEPLOYMENT_EVIDENCE_SHA256 \
  --rollback-sha256 APPROVED_ROLLBACK_EVIDENCE_SHA256 \
  --transaction /absolute/private-evidence/cloudflare-ROLLBACK_ID \
  --execute
```

The transaction activates runtime, signer, and public versions in that order,
restores the exact recorded previous public-host routes, runs the live smoke
suite, and writes only canonical mode-`0600` events under the mode-`0700`
transaction directory. Each intent is durable before its Cloudflare write.
After a crash, rerun the identical command and transaction path; the operator
accepts only the exact next Worker or route prefix and never repeats an already
observed activation. A torn local event write is repaired only when it is the
exact next transaction event and its bytes are an exact prefix of the expected
canonical event. Preserve the printed terminal rollback-receipt SHA-256
independently from the transaction directory. The immutable datadir Worker and
any legacy Worker named by the recorded prior route snapshot must remain at
their exact recorded deployments throughout. Bare, `http://`, and `https://`
Cloudflare route patterns retain their exact spelling in the evidence; see the
[official route-pattern syntax](https://developers.cloudflare.com/workers/configuration/routing/routes/).

A first installation whose evidence records an absent previous runtime,
signer, or public Worker has no byte-exact Worker version to restore and is
deliberately non-rollbackable by this command. Restore its recorded routes only
through a newly reviewed operator procedure; do not guess by deleting scripts.
The Cloudflare token remains in the environment, is redacted from API errors,
and is never written to either evidence directory.

## Protected GitHub workflows

### Production protection

Create a protected GitHub Environment named `cloudflare-production` with the
required reviewers. Configure these values on that environment:

- variable `CLOUDFLARE_ACCOUNT_ID`;
- variable `CLOUDFLARE_ZONE_ID`;
- secret `CLOUDFLARE_API_TOKEN` with only the account Worker-script and zone
  Worker-route permissions needed by these workflows.

All deploy workflows use the environment. They are manual and their
concurrency groups do not cancel a release in progress. A pull request or push
can verify public/signer assets, but cannot deploy them.

### Runtime deployment

Dispatch `deploy-static-runtime.yml` with:

- the source Actions run ID;
- the exact artifact name;
- the independently approved SHA-256 of `runtime-authority.json`.
- the approved datadir authority SHA-256 and exact deployed datadir Worker
  version from `datadir-deployment.json`.

The unprotected job downloads only that named artifact, requires the exact
layout, independently reproduces the inventory, runs `verify:runtime`, and runs
`verify:runtime-wrangler`. It then preserves that exact verified input for the
protected job. After environment approval, the protected job repeats every
check and deploys `robinhood-runtime-assets` without attaching a route.

Wrangler writes a structured deployment record. The workflow extracts its
exact version UUID, proves that version exists under the expected Worker and is
the current 100% production deployment through the Cloudflare API, and stores
the Wrangler record beside the unchanged inventory. Record the printed runtime
version UUID; it is required by the public/signer deployment.

### Public and signer deployment

Dispatch `deploy-static-workers.yml` with `deploy=true` and the exact runtime
and datadir version UUIDs. The protected deployment
performs this order:

1. re-verify the freshly built public and signer origins against their exact
   inventories;
2. create or verify the query-safe `/api*` and permanent HTTP-01 no-script
   bypasses;
3. prove the named runtime Worker version exists and is the current 100%
   production deployment;
4. deploy the freshly built signer and public Workers;
5. apply the complete ordered route authority; route code proves the runtime
   version again before its first route mutation;
6. check the live route closure and smoke the public page, leaderboard deep
   link, isolated signer, runtime manifest/WASM/Demo object, and VPS API bypass.

The signer is always rebuilt from source using the pinned wasm-bindgen 0.2.127
authority. Its ignored local `signer-dist` is never a deployment input. The same
rule applies to ignored `dist` and `runtime-dist` directories: do not run a live
Wrangler deployment from a developer checkout.


### Updates and workflow recovery

First publish the exact datadir and runtime versions, then the public/signer
workflow. A new complete runtime corpus retains every immutable build and
changes only `wasm/latest.json`; never edit a deployed corpus in place.

Protected-workflow recovery requires the preserved previously admitted complete
corpus and its original inventory. Republishing it creates a new Worker version;
use that proven UUID for the public/signer workflow. Local evidence-authorized
rollback instead reactivates recorded old versions without uploading bytes, as
described above. These are distinct recovery inputs. A partial route operation
must stop for investigation; never manually attach a broad route to finish it.

## Host compatibility overlay

This is a release-specific recovery tool, not a general step for every new
release. Source and payload live in
`crates/robin_highscores/deploy/host-compat-overlay/`; retain the original
reviewed identities when using it. It targets the d301 base units and e080
transient drop-ins described below.


This narrowly scoped overlay makes the already-proven VPS compatibility setting
persistent across user-manager restarts and host reboots. It does not replace or
modify an installed release or its three base unit files.

On the production Debian host, exact user-manager probes and a real BackupV4 run
showed that combining `PrivateUsers=yes` and `RestrictSUIDSGID=yes` makes Linux
`openat2` return `ENOSYS`. Robin Hood's descriptor-pinned runtime fence requires
`openat2`, so the three services must explicitly reset only the latter setting:

```ini
[Service]
RestrictSUIDSGID=no
```

The installed d301 release remains immutable. The tool fails closed unless the
API, worker, and backup base units still have their exact reviewed hashes, sizes,
modes, owner, and single-link identity. `PrivateUsers=yes`, `NoNewPrivileges=yes`,
empty capability sets, private devices, kernel protections, syscall filters, and
all other base hardening remain unchanged.

### Sealed artifact layout

Keep the executable and payload in one owner-only directory. The executable must
be mode `0500`; the payload must be mode `0400`, owner/group `robinhood`, and have
one hard link. Its expected SHA-256 is
`832331e248a58932f3f0923285d7979d7316306cd0947d2502408e8e08bef579`.

### Install and verify

Run these commands as the unprivileged `robinhood` account:

```sh
./robin-highscores-host-compat-overlay install-and-retire-transient
./robin-highscores-host-compat-overlay verify
```

Installation creates each absent `UNIT.service.d` as a private staged directory,
installs the exact mode-0400 payload, and promotes the directory with a
same-filesystem no-replace rename. A repeated install is idempotent only when all
existing bytes and metadata are exact. The tool reloads the user manager and
verifies the exact base fragment, the complete `DropInPaths` set, and all material
effective hardening properties for all three units. It then authenticates and
atomically quarantines the exact e080 runtime-only singleton drop-in directories,
reloads, proves that only the persistent drop-ins remain effective, and deletes
no runtime files. The three exact dot-prefixed quarantines are retained under
`/run`: systemd does not load them, they consume 90 payload bytes, and they
disappear together when the runtime filesystem is recreated at reboot. A crash
during quarantine promotion resumes from the exact live/quarantined mixture. A
reload or effective-proof failure restores all three exact transient directories
and reloads them before returning failure. It does not restart services.

### Roll back

Only remove this overlay when reverting the current host workaround or when a
separately reviewed canonical release procedure has taken ownership of the same
compatibility requirement:

```sh
./robin-highscores-host-compat-overlay remove
```

Removal first verifies the complete installed state, atomically quarantines all
three exact directories, then reloads and checks the user manager. It deliberately
retains those fixed-name authenticated quarantines as the rollback artifact,
avoiding a second multi-directory irreversible cleanup phase. A repeated removal
restores, reloads, re-proves, and quarantines them again. If reload or verification
fails, it restores all quarantines and reloads again.

The cutover owns only the three exact runtime-only e080 singleton directories
under `/run/user/1002/systemd/user`. Any unexpected file, directory, link,
metadata, digest, `DropInPaths` entry, or effective hardening value is a hard
failure. After successful cutover no runtime-only directory is loadable: the
retired exact directories are either all quarantined or all absent after reboot.
Removing the persistent overlay therefore restores the base unit's
`RestrictSUIDSGID=yes` on the next service start; use removal only as part of an
explicitly reviewed rollback or canonical successor activation.

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
cargo test -p robin_highscores
cargo test -p robin_highscores --test router_e2e
python3 -m unittest discover -s scripts/release -p 'test_author_leaderboard_release.py'
```

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

Before publication, run `validate-publication-v3`, source-independent
`validate-vps-release-v2`, and the applicable static/operator dry runs. Final
supervisor fixtures require operator-provided source-exact Demo and Full/H12
bundles through `ROBIN_SUPERVISOR_DEMO_FIXTURE_DIR` and
`ROBIN_SUPERVISOR_FULL_H12_FIXTURE_DIR`. Intermediate binaries, historical test
counts or missing fixture inputs are not final release evidence.

### Transaction failure injection

The harness executes unchanged deploy/rollback script bytes in a private
bubblewrap `/home/robinhood`; it does not contact the VPS, real systemd or public
site. Record the immutable `ROBIN_TX_SOURCE_DIR` with results:

```sh
ROBIN_TX_SOURCE_DIR=/absolute/path/to/crates/robin_highscores/deploy \
  python3 crates/robin_highscores/deploy/tests/deploy_transaction_state_machine.py

ROBIN_TX_BOUNDARY=mkdir.unit-stage-root \
ROBIN_TX_SOURCE_DIR=/absolute/path/to/crates/robin_highscores/deploy \
  python3 crates/robin_highscores/deploy/tests/deploy_transaction_state_machine.py \
  DeployTransactionTests.test_deploy_prejournal_preparation_orphans_are_reconciled

python3 crates/robin_highscores/deploy/tests/runtime_fence_sigstop_harness.py -v
```

Boundary selectors accept `EVENT` or `EVENT:MODE`. Real fixture files and
command shims model systemd, capacity, ordering, cancellation, TERM, SIGKILL,
side-effect-then-error and concurrent transactions. Mandatory scenarios include
first deploy, upgrade, same-schema rollback refusal, exact-inode promotion,
retained plan FD/digests, same-OFD lock contention, source consumption, orphan
reconciliation, exact clean-first state, initialized fifth-key/fence recovery,
source/target BackupV4 receipts, and pre-migration source restoration versus
post-migration target-stopped convergence. Unexpected managed-state residue,
raw-root drift or altered authority must fail before mutation.

The outer-wrapper and backup/admin shims model frozen descriptor contracts;
Rust tests own typed parser internals. A shimmed BackupV4 result is an ordering
oracle, not a compiled-binary sandbox proof. The separate SIGSTOP harness uses
real processes and shared/exclusive kernel locks: a backup actor recovers a
stale admission gate, heartbeats while blocked, terminates stopped writers
within bounds, fsyncs its completion and restarts the shared holders.

The candidate's mandatory `real-runtime-fence-release-gate.sh` instead runs the
actual candidate server, worker and BackupV4 admin in isolation before
activation. Neither a skipped state-machine suite nor synthetic lock holders
replace that gate.

### Frozen runtime probes

These candidate-admin interfaces use descriptor-pinned release authority,
not a configuration path:

```text
probe-runtime-authority-v2 --candidate-release-root-fd FD --expected-vps-release-manifest-sha256 HEX --backup-authority-state absent|present
verify-live-database-schema-v2 --candidate-release-root-fd FD --expected-vps-release-manifest-sha256 HEX
```

Each self-attests the candidate admin and emits canonical newline-free JSON
with `schema_version: 2`, `source_commit`, `vps_release_manifest_sha256`, and
respectively `backup_authority_state` or `database_schema_version`. Tests must
prove absent → durable initialize-v2 intent/key → present → complete-v2 intent
removal, and read-only live-schema admission including a committed WAL while
both runtime locks are held. Main-file-only schema reads are insufficient.
