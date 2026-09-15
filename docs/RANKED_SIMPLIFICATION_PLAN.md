# Ranked leaderboard simplification plan

Status: inventory complete (2026-09-15), implementation in progress on branch
`rip-ranked-projection`.

## Decision

We fully control the engine, the verifier build, the raw game content and the
server. Everything that exists only to prove engine/build/content properties to
ourselves is removed. The target is:

> the server receives a replay, the verifier resimulates it with our pinned
> verifier build against our raw game content, checks the recorded state
> hashes and outcome, and scores it.

## What production actually runs today (verified)

- The client only uses the **recorded-replay lane** (commit 43b88b2eb): after a
  single-player mission it encodes the compact replay, fetches
  `leaderboard-metadata`, `content-manifests`, `rules-configs`,
  `published-rulesets` and `builds` to *select* digests, builds an **unsigned**
  `ReplaySessionGenesisV1` carrying those digests, requests an offer, signs the
  envelope with the player key and uploads.
- Production never requests fresh/continuation preflight grants or competition
  grants, never creates an official multiplayer ranked session
  (`install_ranked_session_setup(None)`), and never co-signs with guests. The
  recorded lane only submits `IndividualLevel`, so campaign continuations and
  full-campaign aggregates are unreachable from the client.
- The verifier recomputes an 8-component semantic projection of the loaded
  mission and byte-compares it against a 2.3 GB operator "verifier bundle"
  (`map_geometry_metadata.bitcode` etc.) whose manifests are pinned by the
  offer, before it resimulates from the raw content it just loaded anyway.
- Live data: 280 `admission_profiles`, 0 competitions, 0 submissions, 0
  verified runs (no legacy data to preserve besides identities/diagnostics).

## Target architecture

```
client (native/web)                         server (robin-highscores)
-------------------                         -------------------------
record replay (compact rhrec)
GET  /api/v1/leaderboard-metadata  ───────▶ boards from server.toml
     pick board: mission + edition + loaded SimConfig matches board policy
POST /api/v1/submission-offers     ───────▶ one-use upload challenge
     {uploader key, mission, board_id, artifacts}
sign SubmissionEnvelopeV2 (player Ed25519 key)
POST /api/v1/submissions (multipart: submission, replay, starting campaign)
                                            store content-addressed objects,
                                            queue job
                                            worker: VerifierJobV2 {board policy,
                                              edition, locale root, limits}
                                            bwrap+prlimit robin-replay-verifier
                                              --raw content ro-bind
verifier: bounded compact decode → header checks (mission/seed/campaign bytes)
  → board policy accepts replay SimConfig → starting campaign is an official
  fresh start (profiles loaded from raw content) + structural campaign
  validation → load mission from raw content → Engine::new + ranked policy
  → resimulate, check every state hash, require terminal success
  → metrics/achievements (compiled official catalog) → result + final campaign
                                            accept → verified_runs → boards
GET /api/v1/leaderboards, /runs/{id}, /runs/{id}/replay (public replay bytes)
viewer: picks the runtime build from the replay's recorded engine commit
```

Pinning that remains: the worker launches the verifier at a configured path in
the authority release; boards pin the edition, mission list and simulation
policy; replay schema / network protocol versions must equal the verifier's
compiled constants. There are no manifest digests in offers, sessions, results,
queries or database rows.

A new authority release contains only `bin/robin-replay-verifier` (~30 MB, static
musl) plus the `server.toml` board list. Raw Demo/Full content stays installed
read-only under `raw-content/{demo,full}` as today. No bundles, catalogs,
campaign templates, source-tree manifests, rules/ruleset/policy/build documents.

## Classification

### (A) Remove

| Component | Where | Why it is only proof-to-ourselves / unneeded |
| --- | --- | --- |
| Static content projection (8 components, projection serializer, component documents, `SIMULATION_CONTENT_COMPONENT_ORDER_V1`) | `robin_engine/src/simulation_inputs.rs`, `robin_run_types/src/content.rs` | Re-derives what the verifier already loads from raw content and compares it to an operator copy of itself. |
| Run projection, `PreparedMissionInputs`/`RankedPreparedMissionInputs` capability, `PreparedMissionInputsSealV1`, `prepared_*_sha256`, `admit_ranked_content`, `validate_static_content`, `ranked_protocol_seal` | engine `simulation_inputs.rs`, `engine/rollback_safe.rs`, `robin_run_types/src/session.rs`, `robin_ranked_verification` `engine_preparation.rs` | Seal of the above; recorded lane already sends `None`. Engine construction becomes `Engine::new` + explicit ranked policy install. |
| `ranked_opacity_ready` / `ranked_timing_ready` / `sprite_opacity_sha256` gates | engine | Missing timing/opacity data in *our* raw content would show up as state-hash divergence; not a trust boundary. |
| `ContentManifestV1`, `CampaignContentManifestV1`, `ContentClosureKindV1`, `SimulationContentComponent*`, `SimulationSpeechTimingSourceV1`, `ResourceLocaleRootV1` in protocol, all `Official*` projection/receipt/source-tree/overlay/exporter types, official subject name helpers | `robin_run_protocol/src/manifest/content.rs`, `robin_run_types/src/content.rs` | Content identity documents. Replaced by `(edition, mission_id)` + operator raw-content mount. Locale root becomes per-edition worker config; speech timing is always `CoreAudioDurationsV1`. |
| `BuildManifestV1/V2`, `VersionedBuildManifest`, `Browser*BuildIdentity/Recipe/Policy`, `BuildToolAuthority*`, `RustToolchainAuthorityV1`, `VerifierBuildIdentityV2`, `OfficialViewerBuildReportV2`, `OfficialProjectionAuthorityManifestV2`, `.github/tool-authorities/*` | `robin_run_protocol/src/manifest/build.rs`, `manifest.rs` | Build provenance documents. Compatibility is replay schema + network protocol; viewer uses the replay's recorded engine commit. |
| `RulesConfigIdentityV1` documents, `RulesetManifestV1`, `PublishedRulesetV1`, `RulesetOperationalStatusV1`, `ImmutablePolicyManifestV1`/`ImmutablePolicyIdentityV1`, canonical start/campaign-state requirement documents, tick/score/tie policy enums | `robin_run_protocol/src/manifest/ruleset.rs`, `robin_run_types/src/ruleset.rs` | Content-addressed policy layering. A board is now a plain server-config entry: id, labels, edition, missions, `RankedSimulationPolicyV1` (preset + difficulty, or custom), metrics. Achievement catalog and state-load/restart rules are compiled constants. |
| Canonical campaign state pins, `canonical_campaign_state_path`, campaign-template matrix, `canonical_fresh_campaign_artifact_v1` digest comparison | server config/DB, manifest tool, engine | The verifier checks the uploaded starting campaign directly against a fresh start built from raw-content profiles (kept, see B); no operator artifact is needed. |
| Fresh-run and campaign-continuation preflight grants, run-preflight grant key, `run_preflight_ttl_seconds`, `/fresh-run-preflight-grants`, `/campaign-continuation-preflight-grants`, signer preflight ops | protocol `envelope.rs`, `web.rs`, admin CLI, identity signer, client `service.rs`/`ranked_session.rs` | Never used in production; pre-game authority that the recorded lane replaced. |
| Session genesis as a manifest carrier (`RankedSessionConfigV1` digests, unsigned genesis in recorded lane), `used_replay_session_geneses` genesis bookkeeping | protocol, server, client | Carries only removed digests. Replay uniqueness is enforced by replay digest + replay session id derived from the replay. |
| Verifier job catalog (`VerifierJobConfigCatalogV1`, route/template), catalog sha pin, content catalog mount, `validate_content_mount`, `ordered_documents`, job-config identity cross-checks, verifier self-hash against build manifest, `validate_catalog_covers_server`, `validate_worker_authority_layout`, source-tree manifest loading in the worker | `robin_replay_verifier/src/{content_manifest,job_config}.rs`, `robin_highscores/src/{deployment.rs,bin/worker.rs}` | Operator-authored copies of server config, cross-checked against itself. Replaced by a small per-job `VerifierJobV2`. |
| `ManifestRegistry`, `manifest_directory`, `admission_profiles` with digest ids, `validate_profile` cross-checks, immutable manifest routes (`/builds`, `/content-manifests`, `/campaign-content-manifests`, `/rules-configs`, `/ruleset-manifests`, `/published-rulesets`, `/policies`) | `robin_highscores/src/config.rs`, `web.rs` | Serves and validates the removed documents. |
| Acceptance-time re-validation of build/content/ruleset tuple (`validate_ranked_policy`, route/job-config/policy sha rechecks) | `robin_highscores/src/db/acceptance.rs`, `db.rs` | Re-proves what the worker just configured. |
| Public proof documents that cross-bind manifests (`PublicVerificationRequestV1`, `PublicVerificationProofV1`, `public_projection_binding_json`, `PublicBuildV1`, `RunContentIdentityV1`), web `run-proof.ts`, `build-contract.ts`, `content-contract.ts`, `ruleset-contract.ts`, `policy-display.ts` | protocol `query/public_proof.rs`, DB, `wasm-www/src/leaderboards` | Present manifest bindings; run detail keeps the verified facts (board, mission, metrics, participants, replay digest, verifier build commit). |
| `robin_manifest_tool` crate (manifestctl, sandbox_v3, plan_v3, official content source, release admission, verifier catalog, campaign templates, typed JS authority) and its examples; `goblin`, `wait-timeout` deps | `crates/robin_manifest_tool` | Authoring tool for the removed documents. |
| `projection-export` feature, `export_simulation_content` example, `official_projection_export.rs`, `rust_init_official_projection`, `configure_official_projection_mounts`, `official_projection_strict`, musl `libdl` shim | `robin_rs`, `robin_data_io` | Exporter for the projection bundles. |
| `scripts/release/author_leaderboard_release.py` + tests | `scripts/release` | Renders profiles and catalogs from manifests. |
| `ViewerLaunchV1` build-digest viewer selection, `viewer_engine_build` | protocol, server, web | Replaced by the replay's recorded engine commit. |
| Web deploy proof chain: `datadir-release-authority.mjs` inventory/authority/receipt, `write-/verify-static-origin-inventory.mjs`, receipt binding in `assemble-runtime-corpus.mjs`/`verify-runtime-corpus.mjs`/`deploy-cloudflare.sh`/`sync-cloudflare-routes.mjs --prove-datadir`, `verify-runtime-source-contract.mjs`, `retained-pre-audio-runtimes.json` hash pins | `wasm-www/scripts`, `scripts/release.sh` | Prove release bytes to ourselves; replaced by a plain `datadir-release.json` `{url, sha256, byte_length, native_content_sha256}` that `release.sh` passes to runtime staging. |
| DB columns/tables for removed concepts (`*_manifest_id`, `config_id`, `ruleset_id`, route/job-config/policy sha, canonical campaign state json, public projection json, `competition_run_grants` if competitions go, genesis tables) | `migrations/` | New migration recreates submission/run tables (0 rows live). |

### (B) Keep

| Component | Reason |
| --- | --- |
| Compact replay format, bounded decoder, canonical re-encode check, spellforge/archive rejection | Hostile input boundary. |
| `robin_engine::ranked_resim`, replay rankability/taints, ranked command admission, state-hash coverage | The actual verification. |
| `RankedSimulationPolicy` install + `validate_config` | Board policy enforcement during resim. |
| Starting-campaign checks: `decode_and_validate_replay_campaign`, approved Sherwood metadata derivation, fresh mission-start check (`validate_canonical_mission_start_v1`, refactored to take `ProfileManager` loaded from raw content) | Uploaded campaign is hostile: prevents boosted-stat starts. Drops the manifest-digest `approved_identity`. |
| Upload challenge / offer (one-use nonce, expiry), multipart upload, reservations, storage admission, rate limits, concurrency caps | Abuse limits and replay-of-request protection. |
| Content-addressed replay/campaign stores, GC, backups, ops scripts, migrations runner | Storage. |
| Boards, leaderboard queries, cursors, player profiles/history, run detail, replay download | Product. |
| Usernames, deletion, abuse reports, moderation, diagnostics | Product/abuse. |
| Worker queue, leases, retries, infrastructure-failure classification | Operations. |
| `robin-replay-admission` native helper + wasm admission worker (local playback of untrusted replays) and its build-identity check | Client-side containment of hostile replays; not ranked authority. |
| Multiplayer `content_identity.rs`, join tickets, web `manifest.json` `multiplayerContent`/`javascriptModules`, `preload-assets.json`, `latest.json`, `runtime-contract.json` | Non-ranked multiplayer compatibility and runtime selection. |
| bwrap/prlimit verifier sandbox (pending C5 detail) | Real trust boundary for hostile replays. |

### (C) Asked the user — decided 2026-09-15: all as recommended

All eight items below were accepted as recommended: remove 1, 2, 3, 6, 8;
keep identity + signer origin but trim it (4); keep the sandbox without hash
pins (5); replay schema + network protocol compatibility only (7). For item 8
`scripts/release.sh --web-only [--rebuild-datadir]` must keep working on the
existing staging `live` layout, keep publishing retained Demo generations,
and `scripts/test_release.sh` covers the new flow.

1. **Multiplayer ranked co-signing** (`ranked_session.rs`, `ranked_client.rs`,
   `ranked_port.rs`, `browser_ranked.rs`, named-seat join attestations,
   participant claims, co-sign transport messages, `LeaderboardCoSign*`,
   signer `sign_multiplayer_leaderboard_request`/`sign_named_seat_join`/
   `sign_replay_session_genesis`, ~6k lines). Production never enables it.
   Removing means multiplayer replays are uploaded by the host only, guests are
   anonymous and counted from recorded seat events (what production does
   today). Recommendation: remove.
2. **Campaign chains and full-campaign boards** (continuation offers,
   controller authorization, `CampaignChainReceiptV1`, chain store,
   HQ sessions, `full_campaign_*` tables, aggregate proofs, `db/aggregate.rs`,
   H12 completion evidence, ~5k lines). Unreachable: continuation offers need
   continuation preflight grants the client never requests. Removing means
   only per-mission boards exist until a simpler recorded-replay campaign lane
   is built. Recommendation: remove now.
3. **Competitions** (`CompetitionManifestV1`, competition run grants and grant
   key, `/competition-run-grants`, `competition_run_grants` table, board
   dimension). 0 configured; client passes no competition metadata. Removing
   loses scheduled challenges with pinned seeds. Recommendation: remove.
4. **Player Ed25519 identity and the isolated browser signer origin.** Still
   used for upload authentication, usernames, deletion and owner status.
   Recommendation: keep both; trim signer to username, submission,
   owner-status, deletion.
5. **Executable hash pins in the sandbox launcher** (`bwrap_sha256`,
   `prlimit_sha256`, `verifier_sha256`, memfd sealing of executables).
   Only proves our own host binaries unchanged. Recommendation: keep
   bwrap/prlimit launch and limits; drop the hash pins.
6. **Per-job raw content re-inventory** (`OfficialSourceTreeManifestV2`,
   `demo_/full_raw_content_manifest`) and the read-only-mount/no-symlink walk.
   Proves our raw content unchanged. Recommendation: drop the inventory,
   keep the read-only bind mount into the sandbox.
7. **Exact build pinning of replays to verifier** beyond replay schema and
   network protocol (the old `allowed_build_manifest_sha256`). Already relaxed
   in d74412d9b. Recommendation: accept any replay with matching
   schema/protocol versions; record the replay's engine commit for viewing.
8. **Web deploy proof scripts** listed in (A): they sit in the live
   `release.sh` path. Recommendation: remove, replaced by `datadir-release.json`
   and the existing `smoke-cloudflare-deployment.mjs`.

## Protocol V2 (implementation contract)

All leaderboard documents that change shape move to `V2` names and
`schema_version: 2`; unchanged documents keep their `V1` names. HTTP paths
stay under `/api/v1`.

### Leaf types (`robin_run_types`)

- Keep `ArtifactRefV1`, canonical JSON, digests, validation,
  `OfficialContentEditionV1`, `RankedSimulationPolicyV1` (+ preset/difficulty).
- Add `BoardSimulationPolicyV1 = Fixed { policy: RankedSimulationPolicyV1 } |
  AnyConfig`. `Fixed` admits exactly the preset's expected `SimConfig`;
  `AnyConfig` admits any validated `SimConfig` under a `Custom` policy whose
  difficulty is derived from the config (the old `preset_id = "any"` boards).
- Remove `RulesConfigIdentityV1`, content manifests and components,
  `ResourceLocaleRootV1`, `SpeechTimingAuthorityV1`,
  `PreparedMissionInputsSealV1`, `bitcode_value`, `RANKED_CAMPAIGN_MEDIA_TYPE_V1`.
- `co_sign.rs` and `ReplaySessionTranscriptV1`/seat lifecycle types are
  removed together with multiplayer ranked co-signing (client work item).

### Submission (`robin_run_protocol::submission`)

- `UploadChallengeRequestV2 { schema_version, public_key }` →
  `UploadChallengeV1` (unchanged shape; issued for that key, one use, expiring).
- `SubmissionV2 { schema_version, upload_challenge: UploadChallengeV1,
  uploader_public_key, public_disclosure: ParticipantPublicDisclosureV1,
  board_id: OpaqueId, mission_id, replay: ReplayArtifactV1,
  requested_metrics: Vec<BoardMetricV1> }`.
- `SignedSubmissionV2 { schema_version, submission, algorithm, signature }`,
  signature = Ed25519 over `domain_separated_bytes(
  "robinhood/leaderboards/2/submission\0", submission)` by the uploader key.
- Kept as is: `ReplayArtifactV1`, `RANKED_REPLAY_MEDIA_TYPE_V1`,
  username challenge/update, deletion, abuse reports, owner-status challenge
  and envelope, diagnostics, `SubmissionAcceptedV1`, `PublicSubmissionStatusV1`.
  `SubmissionLifecycleV1::Accepted { run_id }` loses the campaign receipt.
- Removed: offers and offer requests, session genesis, ranked session config,
  participant claims/signatures, join attestations, co-sign, preflight and
  competition grants, campaign continuation/authorization/chain receipts,
  `InitialStateExpectationV1`, starting-campaign artifacts.

### Verifier (`robin_run_protocol::verification`)

- Input `VerifierJobV2 { schema_version, job_id: OpaqueId, edition,
  mission_id, simulation_policy: BoardSimulationPolicyV1, allow_state_load,
  replay: ReplayArtifactV1, resource_locale_root: String,
  limits: VerificationLimitsV1 }`, written by the worker (trusted).
- CLI: `robin-replay-verifier --job PATH --replay PATH --content-root DIR
  --result PATH`. A malformed job is an infrastructure failure (non-zero exit).
- Output `VerifierOutputV2 { schema_version, job_sha256, replay_sha256,
  input_provenance: Option<InputProvenanceStatusV1>, status:
  VerificationStatusV2 }`, `VerificationStatusV2 = Verified(VerifiedRunV2) |
  Rejected(VerificationRejectionV1) | FailedInfrastructure(...)`.
- `VerifiedRunV2 { recorded_engine_version, sim_config: CanonicalValue,
  max_concurrent_players, participant_instance_count, outcome,
  starting_campaign_score, final_campaign_score, original_score_delta,
  final_state_sha256, replay_frames, active_simulation_ticks,
  ransom_collected, achievements: Vec<VerifiedAchievementV1> }`.
- Achievement catalog and Required/Optional modes are a compiled constant
  (`official_achievement_policies()`), not a ruleset document.
- Rejection codes drop `BuildNotAllowed`.

### Boards and queries (`robin_run_protocol::board`, `::query`)

- `BoardV2 { board_id, display_name, edition, preset_id, preset_name,
  difficulty_id, difficulty_name, simulation_policy, allow_state_load,
  metrics, viewer_content_requirement, missions: Vec<BoardMissionV2 {
  mission_id, display_name }> }`; `LeaderboardMetadataV2 { schema_version,
  boards }`.
- `RunFilterV2 { schema_version, board_id, mission_id, metric,
  max_concurrent_players, player_public_key }`; flat `LeaderboardQueryV2`
  adds `limit`, `cursor`.
- `BoardMetricValueV2 = OriginalScore { points } | FastestSuccess {
  active_simulation_ticks }` (tick duration is the compiled engine constant).
- `LeaderboardEntryV2`, `LeaderboardCursorV2`, `LeaderboardPageV2`,
  `RunSummaryV2`, `PlayerRunHistory*V2`, `PlayerPersonalBestV2`: as V1 without
  composition, content/rules/ruleset/competition digests or aggregate
  participants; participants are the optional named uploader
  (`PublicParticipantV1`, seat 0) plus anonymous counts.
- `RunDetailV2 { schema_version, run_id, board_id, mission_id, edition,
  metrics, max_concurrent_players, participant_instance_count, uploader,
  verified_at_unix_ms, replay, recorded_engine_version, sim_config,
  starting_campaign_score, final_campaign_score, achievements, viewer:
  ViewerLaunchV2 { availability, content_requirement, runtime_build } }`.

### HTTP API

Kept: `leaderboard-metadata`, `leaderboards`, `runs/{id}`, `runs/{id}/replay`,
`players/*`, username, deletion, reports, submission public/private status,
diagnostics, operator, health. New: `POST /api/v1/upload-challenges`.
`POST /api/v1/submissions` multipart has two fields: `submission`
(`SignedSubmissionV2`) and `replay`. Removed: `submission-offers`, all
preflight/competition grant routes, all immutable manifest routes
(`builds`, `content-manifests`, `campaign-content-manifests`, `rules-configs`,
`ruleset-manifests`, `published-rulesets`, `competitions`, `policies`),
`runs/{id}/campaigns/*`, `runs/{id}/sessions/*`.

### Server and worker configuration

`server.toml` loses `manifest_directory`, `admission_profiles`,
`competitions`, grant key paths, `campaign_state_directory` and
`run_preflight_ttl_seconds`, and gains `[[boards]]` entries shaped like
`BoardV2`. `worker.toml` loses the catalog, source-tree manifests and all
executable hashes and gains `[content.demo|full] { root,
resource_locale_root }`; `[verifier_launcher]` keeps program paths and limits.
DB migration 0006 drops and recreates submission/run tables (no live rows) and
drops competition, campaign, genesis and aggregate tables.

## Phasing

1. Engine/protocol: delete projection, seal, content/build/ruleset documents.
2. Verifier: `VerifierJobV2`, raw-content-only preparation.
3. Server: board config, offers without digests, worker, acceptance,
   migration 0006, README.
4. Client: recorded submission against boards; delete admission document
   fetching.
5. Authoring tools: delete `robin_manifest_tool`, `projection-export`,
   Python config authoring.
6. Web: leaderboard TS DTOs, viewer build selection, deploy proof scripts.
7. C items after the user's answers.
