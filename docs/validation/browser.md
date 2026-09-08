# Actual browser game validation

Snapshot: `3e6beaa17ace` (production code); diagnostic branch `validation-browser`.
This is **desktop headless Chrome 152.0.7977.64 using software SwiftShader**, not
a physical-device, accelerated-GPU, editor, or production-deployment test.
Final production scope is that pinned snapshot plus approved fixes `90a1dd304`
and `d2c76faeb`. Unrelated changes subsequently merged into main were not pulled
into this worktree; **this is not runtime acceptance of the latest moving main**.

## Final bounded outcome

The two approved fixes are exercised successfully in real Chrome: asset preload
no longer throws an illegal-invocation error, and the first frame no longer
panics in the desktop clock. Actual Leicester scene rendering, live mission
state, pause/manual frame advancement, separate-origin signer isolation, audio
decode/source start, and isolated replay admission are evidenced. **Full game
acceptance is not claimed** because the remaining failures below were reproduced.

| Check | Observed result | Evidence directory under `/tmp/` |
| --- | --- | --- |
| Real rendering and manual advancement | Scene/tutorial parchment; frame 2 → 7, five ticks and one modal dismissal | `robin-browser-validation-VDZtUc` |
| Ordinary input replay export | Compact export succeeds; no uncaught exceptions in that run | `robin-browser-validation-siLE3W` |
| Matching actual replay worker | Exported replay accepted by the real memory-capped wasm worker | `robin-browser-validation-aVfdo3` |
| Correct audio instrumentation | 633 decodes, one actual source `start`, running 44.1-kHz context; no listening claim | `robin-browser-validation-oYrWPQ` |
| Actual signer isolation | Public typed status, raw-key operation rejected, parent DOM access denied, no parent IndexedDB | `robin-browser-validation-H5TkYf` |

Key screenshots are `VDZtUc/game.png`, `siLE3W/playing.png`, and
`oYrWPQ/game.png` in those directories. They visibly show the missing-text
tutorial fallback; they are not evidence of correct localization.

The final extended ordinary-input run (`oYrWPQ`) times out in CDP
`Input.dispatchMouseEvent`, then in reattachment `Runtime.enable`; its renderer
responsiveness/navigation path is inconclusive, not relabelled as success.
The earlier BFCache observations prove persisted document restoration only,
not sustained healthy gameplay after a restore. No further production changes
were attempted for these unrelated failures.

The last bounded isolated worker check (`aVfdo3`) passes with no page exceptions,
no network failures, and only `127.0.0.1` endpoints. Validator wasm hash:
`ba48b9c92346ce331e3bc507245ed8081e7efb62116d9e548171ed701edf7cf0`;
input replay file hash:
`c399bee2b07fee56d712261d8c67629b516506e9acc568602e8d91d6cdb0c600`.
Game replay reload/playback remains unproven; export and worker admission are
separate claims.

## Initial snapshot result and approved corrections

**Acceptance failed on the unmodified snapshot.** Actual Chrome boots the current
wasm module and downloads the real Demo datadir, then rejects asset preloading:
`Failed to execute 'fetch' on 'Window': Illegal invocation`.

- Root cause: `asset-preload.ts` invokes injected `deps.fetch(...)` as a method;
  the browser's native `Window.fetch` rejects the dependency object receiver.
  A real Chrome minimal reproducer returns HTTP 200 for detached `fetcher(url)`
  and the exact exception for `({ fetch: fetcher }).fetch(url)`.
- Unmodified evidence: `/tmp/robin-browser-validation-enRcYV/result.json`,
  `failure.png`, `cdp-events.jsonl`, `requests.jsonl`. An earlier independent
  unmodified failure is in `/tmp/robin-browser-validation-BzG7wH/`.
- The user subsequently authorized a narrow production fix: detach the fetch
  function before both manifest/asset calls (`90a1dd304`). A receiver-sensitive
  regression test covers both paths. Pinned `pnpm verify:web` passes: 36 top-level,
  83 leaderboard, 4 shared identity and 243 deployment tests, typechecking and both
  production shell builds. The no-injection browser retest confirms this
  regression is fixed: `/tmp/robin-browser-validation-U7hXUY/` loads all preload
  assets and reaches actual mission bootstrap using the rebuilt production shell.
  It then reproduced the pre-existing first-frame Rust clock panic below, without
  modifying the browser's fetch function. This motivated the separately approved
  clock correction.

The no-injection retest screenshot `U7hXUY/game.png` is black, not gameplay proof.
Its `result.json` records the Rust stack, frame-0 state, 496 audio decodes,
a timed-out replay/control request, and only loopback network
endpoints. The harness now explicitly exits nonzero when its recorded failures
or uncaught game exceptions are nonempty (the initial harness recorded these
failures but did not propagate all of them to its exit status).

## Diagnostic-only continuation (not production acceptance)

`--bind-fetch-diagnostic` binds the browser global before importing the unchanged
production bundle. This independently isolates the receiver failure and exposes
the next issue; it is explicitly marked in `result.json`.

Evidence `/tmp/robin-browser-validation-4J0PfM/`:

- Current runtime instantiates; `Dem_Lei_MP` loads its real shipping closure
  (66 fetched files, 15,057,563 bytes); engine reports 199 entities.
- Mission RPC reports map `leicester`, frame 0; the recorder initializes.
- Web Audio decodes 496 items; one 44.1-kHz context resumes. **The initial zero
  start counter is not evidence of absent playback**: its instrumentation wrapped
  `AudioContext.createBufferSource`, while Rust constructs `AudioBufferSourceNode`
  directly. The final harness instruments that node's `start` prototype method.
- First-frame Rust panic: `game_session::terminal_debriefing::mission_completion_clock`
  calls `std::time::SystemTime::now()` (`terminal_debriefing.rs:37`), unsupported
  on wasm32-unknown-unknown. The screenshot is black; **rendering and gameplay
  have not passed**. Pause/replay operations cannot complete after the panic.
  This clock bug **predates the refactor**: the call/helper already exist in
  `96d4a33d9^`, with blame to `cd90a682a9` (August 28). It is newly exposed by
  browser execution, not an introduced refactoring regression.
- Actual navigation reports `pagehide.persisted=true` followed by
  `pageshow.persisted=true`, with null not-restored reasons. This proves a BFCache
  restore occurred, not that a healthy game continued across it.

## Approved clock fix and remaining findings

`d2c76faeb` changes only the mission completion clock to the existing
`web_time::SystemTime`/`web_time::UNIX_EPOCH` and adds a same-module epoch-unit
consistency test. Optional/error/conversion semantics are unchanged. Exact
committed-source wasm rebuild and `cargo fmt --check` pass. The native validation
track also passes **5/5** focused tests (clock consistency plus four debriefing
tests), using `CARGO_BUILD_JOBS=1 cargo test --locked -j1 -p robin_rs --features
desktop --lib game_session::terminal_debriefing::tests` on the pinned baseline
plus this clock commit, in its separate native build graph.

- New game wasm SHA-256:
  `5a56bdc3bc061c63f71e81b46e8e1a177db352769d28d4b43863c3753d3b3767`.
- `/tmp/robin-browser-validation-VSa424/game.png` proves actual scene/modal
  rendering after the clock fix, without fetch injection. That run used the
  developer forced `--mission` route, which bypasses ordinary Demo team creation
  and immediately lost. The harness now defaults to normal Demo auto-start;
  forced mission selection is explicit, not the acceptance default.
- Normal Demo `/tmp/robin-browser-validation-VDZtUc/` renders the scene and
  tutorial parchment, pauses at frame 2, and manually advances five frames to 7.
  Export then fails: `replay frame 3 starts at timeline 7, previous frame ended at
  2`. Read-only inspection suggests `run_forward_ticks` updates rewind history
  without mirroring manual ticks to the active recorder; existing tests use
  `recorder: None`. This omission also exists at `a413e4c80`; no historical runtime
  reproduction was performed, and it is not labelled refactor-introduced.
- The pause UI subsequently panics at `ingame_menu/briefings.rs:92` because
  localized short briefing string 0 is missing. The throwing check predates the
  refactor (`724d0d9ff3`, August 28), but the root cause of absent text may involve
  retained corpus/context compatibility and is not established. Normal startup
  also warns that Ferris is absent and displays `No popup texts for the current
  level!`. Neither missing content nor pause/recorder behavior was changed here.
- `/tmp/robin-browser-validation-siLE3W/` isolates ordinary pointer input without
  manual stepping: replay export succeeds and there are no uncaught exceptions.
  Its replay worker correctly rejects the then-stale helper's `3e6beaa17ace`
  identity against the rebuilt game's `d2c76faeb236`. **That mismatch was a local
  harness staging error, separate from the manual-step recorder failure.** The
  helper was rebuilt at the matching commit for the final run.

## Independently passing browser boundaries

- `/tmp/robin-browser-validation-H5TkYf/`: real production signer shell plus the
  current compiled signer wasm, on a separate mapped HTTPS origin. The real
  sandboxed iframe rejects parent DOM access; typed status returns a public key;
  a prohibited private-key export operation returns `invalid_operation`. Parent
  origin IndexedDB has no databases. Only local ephemeral-profile identity state
  is created; no remote account/session or network write occurs.
- Exact checked-in production headers produce `crossOriginIsolated=false`.
  `/tmp/robin-browser-validation-z63Qlr/` separately adds COOP/COEP and observes
  `true`; this opt-in diagnostic is **not** a claim about deployed headers.
- Both secure-context probes expose WebGPU and a SwiftShader fallback adapter.
  The actual game logs `backend=Gl type=Cpu`, not a WebGPU rendering backend.

## Provenance and safety

- Game and signer built from current source with `CARGO_BUILD_JOBS=1`, pinned
  nightly 2026-08-25, `--locked -Zbuild-std=std,panic_abort`, target
  `wasm32-unknown-unknown`, `wasm-dev`. Game features: `audio`; signer features:
  `identity-signer-bridge`, separate Cargo invocations. No Cargo target override.
- Exact wasm-bindgen 0.2.127 used from
  `/tmp/robin-release-eb595ef7f-wasm-bindgen-0.2.127/bin/wasm-bindgen`; the default
  PATH tool is 0.2.126 and was deliberately not used. Public/private generated
  JavaScript role closures staged and the signer bridge verifier passed.
- The separately memory-capped `robin_replay_admission_wasm` target also builds
  successfully with `scripts/replay-admission-wasm.cargo-config.toml`; its browser
  worker accepts the matching ordinary-input replay in the final isolated check.
- Production shell bundles built with Node 24.19.0 and pnpm 9.15.0.
- Initial game wasm SHA-256: `7a2de38fb421315413fa96d7960d898cf92c379b951a337fd68e254586d7d019`.
  This is an unoptimized validation runtime, not the deployed release artifact.
- Real retained Demo corpus was copied/reflinked, without modifying its source,
  from `/home/phire/.local/share/robin_hood/deployment-staging/datadir-f811ab125-live/datadir-dist/datadirs/demo-leicester/`
  to this worktree's `target/browser-data`. All **95** manifest entries verify
  byte length and SHA-256. The corpus manifest's original engine provenance is
  `ef2928a2c1aff1e7c9be81ae111dd2585cabd6bc`; this is retained data, not falsely
  relabelled newly converted data. Datadir hash:
  `4b6e0fcba222b5df2b640ecf630d5101b94c4eae68bd0f9518f96658e28fd742`.
- Harness uses an ephemeral local TLS server, exact production-named origins
  mapped to loopback, no proxy, and CDP interception rejecting every non-GET or
  non-fixture host/protocol request. The server independently denies writes and
  unknown hosts. Observed network endpoints are only `127.0.0.1`.
- Own Chrome and server processes are bounded and terminated. New harness runs
  remove their generated browser profile and TLS private key, retaining only
  logs, screenshots, public certificate, replay evidence and results.

## Reproduction

From this worktree after the documented build/staging steps:

```sh
node scripts/validation/browser-game.mjs --data target/browser-data --runtime-source d2c76faeb --natural-play --timeout 600000
node scripts/validation/browser-game.mjs --probe-only --signer --runtime-source 3e6beaa17 --timeout 60000
node scripts/validation/browser-game.mjs --probe-only --runtime-source d2c76faeb --replay-file /tmp/robin-browser-validation-siLE3W/replay.json --timeout 60000
```

`--isolated` adds diagnostic isolation headers. `--bind-fetch-diagnostic` modifies
the test browser global only and must never be described as production acceptance.
No live public deployment, service writes, physical input devices, audio listening,
or physical mobile/GPU compatibility was tested.

### Preserved reproduction bundle

Before worktree cleanup, the exact current runtime package (including the matched
replay helper), verified Demo closure, production site and signer shells, preload
assets, deployment headers, harness and selected evidence were copied to
`/tmp/robin-browser-repro.qmpnNx` (216 files, 281,970,242 bytes before the checksum
inventory). `provenance.json` records full baseline/fix/runtime commit IDs and the
original retained-data provenance; `SHA256SUMS` inventories every other file.
Verify the preserved copy with:

```sh
cd /tmp/robin-browser-repro.qmpnNx
sha256sum --check SHA256SUMS
```

The bundle mirrors repository-relative artifact paths. For reproduction, restore
those artifact directories into a checkout containing the documented pinned
source and harness; do not substitute latest-main artifacts. Use the bundled
`evidence/robin-browser-validation-siLE3W/replay.json` for the replay-worker probe.
Original evidence JSON retains its original absolute worktree paths, but the
corresponding files are preserved in this bundle. Its report snapshot is commit
`95f1201f2`; this preservation note is a later documentation-only addition.

The current signer shell no longer includes the generated bridge after the final
site build. The originally tested raw `leaderboard_identity_bridge.wasm` is also
preserved under its original target path; rerunning signer acceptance requires
generating/staging it with wasm-bindgen 0.2.127 as documented above. This does not
claim that the preserved shell alone is the complete earlier signer fixture.
No browser profile or TLS private key is included. `/tmp` retention is local and
temporary, not a durable published archive; preserve this bundle elsewhere before
system temporary-file cleanup if long-term reproduction is required.

The worktree may now be removed without losing these runtime/data artifacts.
The only Rust production change in this pass is the explicitly approved narrow
clock correction.
All owned Chrome/server/build processes have completed; the separate native
validation track has confirmed the focused native debriefing tests pass. The
browser harness passes `node --check`; `git diff --check` passes. No new browser
or production test run is left running by this track.
