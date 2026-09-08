# Browser content and runtime acceptance repair

## Scope and cause

This follow-up starts at `ecfdfbb10`, including the previously approved browser
fetch and clock fixes plus intervening main changes. Earlier acceptance at
`3e6beaa17` plus those fixes is not evidence that this newer snapshot passed.

The retained Demo closure is copied read-only from
`/tmp/robin-browser-repro.qmpnNx/target/browser-data`; the original deployment
staging and installed game data are not modified. Its manifest retains original
engine provenance `ef2928a2c1aff1e7c9be81ae111dd2585cabd6bc` and datadir SHA-256
`4b6e0fcba222b5df2b640ecf630d5101b94c4eae68bd0f9518f96658e28fd742`.

The earlier browser log requested the correct Demo descriptor `RHLevelSB.red`.
The original Leicester Demo installation stores that file in shared
`DATA/Text`, while locale `1033/data/Text` contains `Level.res` and dialogue
audio but no RED descriptor. `localized_level_descriptors` previously consulted
only the active locale map, dropping the shared resource-index metadata. The
popup therefore displayed its missing-descriptor warning and pause objectives
eventually reached the required-text panic. There is also a key-convention
mismatch: the converter stores shared RED filenames with original mixed case,
but the runtime previously requested lowercase keys.

The repair gives the selected locale descriptor precedence, then resolves shared
metadata case-insensitively. Duplicate case aliases are rejected as ambiguous.
Shared descriptors with embedded custom authored strings cannot cross a locale
boundary. Actual text/resource lookup remains strictly localized; missing text
is not replaced by English, empty strings, or numeric placeholders. No converter
change, payload regeneration, schema change, or panic suppression is required.

## Focused validation

- Focused descriptor tests cover the Demo overlay shape, localized overrides,
  missing translated resource boundaries and embedded authored-text rejection.
- An explicitly opt-in test reads the retained real Demo blob and requires both
  actual localized popup and short-briefing string zero to resolve.
- `CARGO_BUILD_JOBS=1 cargo test --locked -j1 -p robin_assets --lib descriptor`
  passed 5 tests (four new resolver cases plus the frozen shipping contract),
  with the real-data case explicitly ignored in this invocation.
- The opt-in real-data case passed 1/1 with
  `ROBIN_BROWSER_CONTENT_FIXTURE=/tmp/robin-browser-repro.qmpnNx/target/browser-data/v8-web-opus-q80.rhdata.zst`
  and `cargo test --locked -j1 -p robin_assets --lib retained_demo_descriptor_resolves_actual_localized_popup_and_briefing -- --ignored`.
- The content production fix is commit `57556377f0aa3d542306363158dbd496dc891bc6`.
  Pinned Node 24.19.0 / pnpm 9.15.0 frozen install and `pnpm build:site`
  passed, including TypeScript checking; emitted game bundle remains
  `game-Cf59yegP.js`.
- Browser harness confirmation now uses the authored Return binding instead of
  coordinates specific to the old fallback popup. Keys are held for 500 ms so
  held-state gameplay actions observe them; release is queued without awaiting
  CDP acknowledgments. Startup watches console events instead of requiring page
  evaluation during synchronous loading. The explicit CDP timeout is 120 s.
  Uncaught game exceptions now reject pending RPC promptly rather than waiting
  for a misleading timeout. BFCache checks require a responsive post-restore RPC.

## Actual browser results and additional pacing repair

All runs use Chrome 152.0.7977.64, headless Linux with SwiftShader software GPU,
production headers (`crossOriginIsolated: false`), no bound-fetch injection, and
loopback-only fixture network access. This is not physical-device testing,
audible listening, or optimized release performance validation.

- `/tmp/robin-browser-validation-CeVeec`: first content-fixed attempt reached
  mission audio warmup but hit the former 45-second startup page-evaluation
  timeout. Retained as a harness timing failure, not proof of a content failure.
- `/tmp/robin-browser-validation-wNXsaF`, source `57556377f`: actual authored
  Robin/Scarlet intro text and portrait render in `game.png`; the log confirms
  the shared descriptor loaded. After Return dismissed the popup, more than 100
  ticks ran but RPC stopped responding for 120 seconds. The graphical pacing
  tail awaited only when frame budget remained. Repeated over-budget frames
  therefore monopolized the browser task, starving events. The actual frozen
  game/helper pair is retained in this directory's `runtime-57556377f`.
- Approved correction `64a963a2c` adds the existing `yield_to_runtime().await`
  when no frame budget remains. It does not alter tick ordering or the native
  refresh path; that helper is a no-op on the native game thread.
- `/tmp/robin-browser-validation-bapggc`, source `64a963a2c`: ordinary input
  advanced frame 2 to 27; replay export was accepted by the matched worker;
  BFCache restored the document with `persisted: true` and live frame 160 RPC.
  There were no page exceptions, failed network requests, or required-content
  errors. Audio instrumentation counted 634 decodes and two real source starts.
  Its `runtime-64a963a2c` preserves the matched binaries. Inspection caught two
  harness limitations: same-frame Escape did not open pause, and load-replay
  merely staged bytes. Neither was counted as actual pause/playback acceptance.
- Intermediate combined source `83f87b1d1`: actual game and matched helper builds pass;
  pinned `pnpm verify:web` passes all browser, leaderboard, shared identity and
  deployment suites, TypeScript checks and both production shell builds.
- `/tmp/robin-browser-validation-ORDfRC`, source `83f87b1d1`: pause frame 2,
  step-forward five frames to 7, and exactly one popup dismissal succeed.
  The 351-byte exported replay (nine recorder frames) is accepted by the real
  matched wasm worker. `replay-restart-menu.png` visibly shows the authored
  rescue objective and Restart selected: the missing-short-briefing panic is
  fixed. The frozen pair is preserved under `runtime-83f87b1d1`.

Actual Restart then exposed a distinct playback activation assertion in
`game_session/replay_init.rs:175`: pending replay must be consumed before Engine
construction. That run failed playback; its subsequent persisted BFCache event
does **not** establish a working restored game (post-restore RPC timed out).
The uncaught assertion and stack are preserved in `cdp-events.jsonl`. The replay
owner subsequently fixed the direct Demo restart entry in `2fd1da96e`, consuming
pending replay data through canonical preparation before Engine construction.

Manual-step replay recording is owned by the separate replay repair track;
native modal scheduling is owned by the native UI track. Neither is silently
included in this content fix. All browser network access remains loopback-only.

## Final frozen-source acceptance

The final combined production snapshot is
`51db7aa06099b54481eb1124df8fa04dd8af6a68`, including the content and pacing fixes,
pending replay activation, manual-step recording, canonical bootstrap and modal
scheduling corrections from their respective owners. These results describe that
exact source, not an arbitrary later main checkout.

Both actual wasm binaries were rebuilt with `CARGO_BUILD_JOBS=1`, the pinned
toolchain, `--locked`, target `wasm32-unknown-unknown` and profile `wasm-dev`.
The game uses `--no-default-features --features audio`; the matched replay helper
uses `scripts/replay-admission-wasm.cargo-config.toml`. Both use
`-Zbuild-std=std,panic_abort`. wasm-bindgen 0.2.127 generated their web bindings;
the engine identity role staging removed the private vault snippet.

`pnpm verify:web` passed all 366 tests (36 browser, 83 leaderboard/signer,
4 shared identity and 243 deployment), type checking and both site/signer-shell
builds under Node 24.19.0 and pnpm 9.15.0. Full captured test/build output is
`/tmp/browser-repair-verify-51db7aa06.log`.

The manual run `/tmp/robin-browser-validation-ZDYKJm` passed: ordinary Demo boot,
pause at frame 2, five forward steps to frame 7 with one recorded popup dismissal,
351-byte export accepted by the actual matched worker, and real menu Restart
followed by playback EOF 9/9. Authored intro and rescue objective are visible in
the screenshots. BFCache emitted persisted pagehide/pageshow and the restored
game answered RPC with its intact EOF state. No page exceptions, content errors,
network failures or replay divergence logs occurred; all observed endpoints were
127.0.0.1. The harness explicitly rejects logged desync even when playback reaches
EOF, because runtime hash validation can log rather than throw.

The normal-input run `/tmp/robin-browser-validation-KgqdOx` also passed. Held
Return dismissed the authored popup and ordinary simulation advanced frame 2 to
59. Held Escape opened the actual pause menu with the localized rescue objective
(visually inspected `pause.png`), then returned to gameplay. Instrumentation
observed 636 successful audio decodes and five real source starts with a running
audio context. Its 781-byte, 218-record replay passed matched admission, actual
Restart activation and EOF 218/218 without logged desync (engine frame 77).
Persisted BFCache restore answered RPC at that same intact EOF state. Exceptions,
content/replay errors and network failures were empty; endpoints were loopback.

The final game SHA-256 is
`70cdf76bc42a142c7b4f2e0e802e0fbda3a8b5c2ff57f99abb87b404be235e46`;
the matched replay helper SHA-256 is
`47fa9227c7fece18a9ff3c0c7c6f5b7ba35cdce8967596d9d91046842716f73e`.
Both exact binaries/bindings, original 95-file Demo closure, production site and
signer shell, core assets, headers, harness, test log and both final browser runs
are preserved outside the worktree at
`/tmp/robin-browser-final-51db7aa06.1bQUPY`, with `provenance.json` and
`SHA256SUMS`. Evidence retains original worktree paths for historical fidelity;
the equivalent relative paths in this bundle are the preserved copies.

These are headless desktop software-GPU checks with production headers, not
physical devices or audible listening. They do not certify browser thumbnail
capture: the software run logs GPU capture polling timeouts while autosave still
commits. No signer bridge runtime, public service write, deployment, or full
mission completion was exercised in this final game pass. The signer shell is
preserved for completeness, not represented as a separately validated final
signer runtime. All owned browser/server processes finished; temporary browser
profiles and TLS private keys were removed by the harness.
