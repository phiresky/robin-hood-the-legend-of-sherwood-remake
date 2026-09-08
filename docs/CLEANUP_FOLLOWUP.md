# Architecture cleanup continuation

Base: `9bed99516`. This pass continues three concrete gaps from
[`ARCHITECTURE_REFACTOR.md`](ARCHITECTURE_REFACTOR.md), using three isolated
implementation worktrees and an integration lane. Performance worktrees and
untracked original sources remain outside its scope.

## Scope and acceptance

| Boundary | Intended change | Acceptance |
| --- | --- | --- |
| Multiplayer server seats | One cohesive active-seat record instead of parallel metadata maps | Preserve authenticated owner/generation checks, detached writers, active replacement, disconnected reservations, ranked readiness and deterministic connection ordering |
| Portrait/HUD uploads | Renderer-local typed borrowed handles backed by explicit upload ownership | Transactional reload, retirement on owner exit, wrong-renderer rejection, optional artwork and identical draw/layout behavior |
| Browser protocol execution | Execute shared admission/framing/reconnect and identity tests in the real browser gate | Keep audio execution, require actually passing tests from each group, preserve native coverage and fail closed on missing browser cases |

No deliberate gameplay, save schema, wire format, or ranked eligibility policy
changes. Keep exact-build replay admission and retain test executables with their
actual source checkpoint. Existing native/browser durable-identity distinctions
must not be removed to simplify tests.

## Review notes

- Active seat replacement retains simulation connection membership while
  clearing readiness and replacing the writer/generation. A detached writer is
  not the same as a fully released authenticated seat.
- Sorting deterministic seat connection publication remains explicit even if
  the backing data structure changes.
- Portrait retirement belongs beside the renderer; adding `Drop` to the outer
  mission can prevent moving its simulation runtime into the mission outcome.
- Actual browser execution must be distinguished from WASM compilation; this
  pass closes the previous shared-protocol and identity execution gap.
- Draw-time portrait provenance checks remain constant-time; complete owner-bank
  validation happens before replacement/retirement, not on every frame. Required
  artwork errors preserve the previous cache, whereas absent optional artwork
  clears its stale slots on successful replacement.
- Requirements-table uploads use the same validated RGB565 upload path as the
  other portrait assets, without collecting duplicate pixel buffers or
  collapsing sparse subframe indices.

## Results

All three tracks passed combined acceptance. Production source checkpoint:
`b1d1ea032da1dc86ab1d80006a6cff6f2cc19052`. The only subsequent Rust change is
`91e4753df`, correcting one test-only RGB565 pixel expectation; integration
checkpoint `cc720ea2a` includes it. No production behavior changed after the
accepted binaries and browser module were built.

### Implemented boundaries

- **Server registry** (`715029976`, `774afe709`): eight parallel active-seat
  maps/sets become one cohesive `ServerSeat` record. Detached writers retain
  authenticated ownership; only the matching owner/generation releases a seat.
  Replacements retain simulation membership but reset readiness. Generation
  overflow cannot consume a reservation, and readiness for a released seat is
  an explicit protocol error. See [registry details](cleanup-peer-registry.md).
- **Portrait ownership** (`1dfe58a9d`, `3bb494ba9`): managed uploads have unique
  retirement tokens and renderer-local borrowed handles. Reload is transactional;
  mission teardown retires the bank beside its renderer. Required PNG failures
  preserve the old cache, successful reload clears missing optional slots, and
  localized/generated names survive replacement. See
  [portrait details](cleanup-portrait-owner.md).
- **Browser execution** (`6ef44e07d`): the gate links `audio,multiplayer` and
  executes the existing shared protocol and browser identity tests in Chrome.
  Its parser requires actual passing cases from each group and reconciles them
  with the runner total. See [browser details](cleanup-browser-protocol.md).

### Combined acceptance

| Suite | Result |
| --- | --- |
| Default client library | 1,568 passed, 6 intentional ignores |
| Release-feature client library | 1,663 passed, 6 intentional ignores |
| Client integration / doctests | 33 integration passed, 4 corpus-dependent ignores; 7 doctests passed |
| Default and release-feature native binary builds | Both passed, built separately from runtime |
| Named Vulkan gate | Passed at `91e4753df`; ownership, public PNG reloads, failed candidates, same-number foreign handles, retirement and queued-draw readback |
| Named browser gate | Both WASM bin/test checks, linked module and 24 real Chrome tests passed; no failures/ignores |
| Gate/dispatch/save-load helper tests | 17 + 13 + 13 passed |
| Formatting / whitespace | `cargo fmt --all -- --check` and `git diff --check` passed |
| Headless and graphical live multiplayer | Both passed all 10 checks; zero desyncs or missed hash comparisons |
| Named native lifecycle gate | All four phases passed: ordinary and save/load recordings, each replayed headlessly and graphically |

Browser execution includes all seven shared protocol cases and both identity
cases, alongside six audio cases and nine existing persistence/cache/HTTP cases.
Chrome/ChromeDriver: `152.0.7977.64`; wasm-bindgen runner: `0.2.127`.
Evidence: `/tmp/robin-lifecycle-gate-pzb_3ngx/summary.json`, `browser-tests.log`
and `browser-tests.wasm`. The clean source checkpoint remained unchanged for
the entire gate. WASM SHA256:
`2d00639dbc0a7ce814432fe778bd04cd8c25f824faf93b6f5f32dcf97ffd4453`.

Native evidence and executable installation are retained in
`/tmp/robin-cleanup-final.a9AALk`, outside disposable worktrees:

- `robin`: release-feature binary, source `b1d1ea032`; SHA256
  `8c8cb39066961ea779a2bf7988e17ef2b75d057c3cbeec80441225255478b897`.
- `robin-default`: default-feature binary, same source; SHA256
  `016601eb508d8aec6c8fd1550660638b1fdefafac112c0e9b1843e1176532ec2`.
- Adjacent `assets/core-datadir` and `mods` support the retained executables.
- `multiplayer-headless/summary.json` and `multiplayer-graphical/summary.json`
  establish both-seat commands, late-input rollback, in-process snapshot
  reconnect, retained seat state, post-reconnect input and periodic hash agreement.
- `native-lifecycle/summary.json` records frozen harness `cc720ea2a`, binary
  source `b1d1ea032`, paused/unpaused stepping, native save/load restoration,
  continued recording, compact export and post-restoration replay hashes.

These native checks used the Leicester Ecoste demo, isolated loopback network
namespaces, temporary player data and virtual displays. Replays were recorded
and played with the same binary; exact-build admission was not bypassed.

### Failures encountered and resolved

- Shared sccache connection resets interrupted the default binary dependency
  build. Retrying with `RUSTC_WRAPPER=` passed; the shared daemon was not restarted
  or reconfigured. The release build's cache warnings fell back locally and
  completed successfully.
- The new GPU readback assertion initially expected full 8-bit white. Existing
  RGB565 expansion yields `(248, 252, 248, 255)`. Correcting that test-only
  expectation made the complete Vulkan gate pass; renderer behavior was unchanged.

### Remaining limits

- TODO: audit generation authorization across every server-reader message.
  This pass preserves existing per-message policies; cohesive storage is not
  a claim that the entire server state machine has been redesigned.
- TODO: migrate remaining integer-based UI upload owners. A standalone portrait
  cache outside `MissionPresentation` still needs explicit retirement while its
  renderer remains live; renderer teardown releases abandoned resources.
- Browser execution establishes shared protocol decisions and resource lifetimes,
  not live remote browser multiplayer, audible output, or full-game browser GPU
  coverage. Native reconnect evidence is same-process, not process-restart
  identity durability. Runtime audio was disabled.
- Standalone engine/service/corpus suites were not rerun: their source was not
  changed. This is a bounded continuation, not another whole-repository rewrite.

### Integration and recovery

Only the four `cleanup-*` worktrees from this pass are cleanup targets. Their
commits remain in main's ancestry and in
`/tmp/robin-cleanup-final.a9AALk/cleanup-branches.bundle` (incremental prerequisite
`9bed99516`). Unrelated performance worktrees and untracked `original-code/`
remain outside scope. No remote push or deployment is part of this pass.
