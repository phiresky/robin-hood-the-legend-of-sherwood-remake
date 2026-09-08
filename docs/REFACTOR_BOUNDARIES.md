# Session, GPU and mission-loading boundaries

Base: `930eebbbc`. This pass implements the four follow-ups proposed after the
[previous cleanup](CLEANUP_FOLLOWUP.md), grouped into three implementation tracks.
An independent read-only audit checks initialization order and network locking.

## Scope and acceptance

| Track | Boundary | Required evidence |
| --- | --- | --- |
| Networking | Authenticated session authorization and cohesive server protocol transitions | Replaced/released/detached readers cannot mutate successor state; preserve legitimate ranked admission, barriers, reconnects, ordering and wire format |
| GPU | Typed mission/menu handles and encapsulated sprite banks | No integer bypass in migrated draw paths; preserve sparse frames and fallback geometry; transactional upload ownership and actual lifecycle retirement |
| Mission loading | Preparation, construction and presentation stages | Owning stage results enforce order; preserve RNG/startup/callback/ranked timing, campaign recovery, headless behavior and terrain handoff |

The main integration lane owns review, combined tests, runtime acceptance and
the final merge. Worktrees have isolated default Cargo target directories;
compilation and runtime are separate. Source checkpoints remain frozen while
builds or acceptance gates run. No shared compiler-cache daemon changes.

## Concurrent work

The active performance branches overlap mission-loading sources. Their changes
are inspected for integration seams, not imported opportunistically. This pass
does not edit or remove those worktrees, and will preserve changes that reach
main before integration. Untracked `original-code/` remains untouched.

## Results

### Review anchors

- Session validation must precede ranked readiness resolution and remain valid
  through local publication and transport effects. Replacement, reader teardown,
  detached writers, handshake opening and timeout callbacks share that boundary.
  Lock order is authority gate, ranked lifecycle, then peers; no guard across
  asynchronous waits or shutdown joins, and no nested gate acquisition.
- Required snapshot acknowledgements are not silently removed by a disconnect.
- Mission staging preserves audio/opacity preparation before engine inputs seal,
  the final authoritative network seed/configuration, browser inline work, and
  interactive deferred terrain join versus headless immediate join.
- Typed menu teardown belongs on the frontend owner, where menu resources and
  renderer are both live, not the outer mission whose runtime moves into an exit
  outcome. Standalone main-menu renderer lifetime already encloses its resources.

### Implemented boundaries

- [Session authority and protocol state](REFACTOR_SESSION_PROTOCOL.md): one
  synchronous authorization boundary spans validation and publication. Four
  cohesive components own readiness, snapshot transitions, co-sign requests and
  ranked admission. Stale readers cannot mutate a successor's state.
- [Mission GPU resources](REFACTOR_GPU_BANKS.md) and
  [menu sprite banks](REFACTOR_MENU_BANKS.md): migrated draws use renderer-bound
  handles; candidate replacements validate before retiring old resources.
  Private sprite banks preserve sparse frames, fallback geometry and unique
  ownership. Lazy menu lookups validate provenance in constant time.
- [Mission loading](REFACTOR_MISSION_STAGES.md) now consumes explicit preparation, engine-construction and
  presentation-attachment stages. Failed construction returns the original
  campaign allocation; stage deserialization cannot recreate runtime authority.
  Interactive terrain remains deferred until presentation upload; headless
  terrain joins before runtime construction.

Independent review found a networking cancellation race: rejecting a buffered
message after detachment could cancel the writer before its queued transition
commit reached the peer. The fix preserves the original pinned writer for a
bounded drain after any typed inactive-session outcome. The regression exercises
the production dispatcher and I/O driver for commit, reconnect, replacement and
release, plus a stalled-writer timeout. This tests an already-started waiting
writer, not a partially transmitted QUIC frame. EOF/error classification was
code-reviewed rather than covered by a dedicated test. No remaining review
blockers were identified.

### Acceptance checkpoints

- `098f00021`: 42 focused release-native networking tests passed.
- `d120b1a39`: full default-client tests passed, including 33 integration tests
  and seven doctests. Named Vulkan execution passed, including actual menu and
  mission ownership, foreign-handle rejection, reload and queued retirement.

### Final combined acceptance

All final builds and gates used clean, frozen source
`09a2438b125fea8451af86e67b3242f408e5653f` (tree
`36855525cb0ed0a6df05b67f343c87fc7971c082`). Subsequent commits only finalize
these reports. Cargo used each worktree's ordinary isolated target directory,
one build job and no compiler wrapper; no clippy or build configuration changes.

| Check | Result |
| --- | --- |
| Full default `robin_rs` package tests | 1,569 library, 33 integration and seven doctests passed; six library and four corpus tests ignored |
| Full release-feature library tests | 1,673 passed, six ignored |
| Named Vulkan execution | Passed, including mission/menu ownership and actual retirement |
| Tools/projection-export examples | Passed |
| Separate default and release-feature binaries | Both built successfully |
| Named browser gate | Both WASM checks, module link and 24 real Chrome tests passed |
| Headless and graphical live multiplayer | Ten checks passed in each; rollback/reconnect and post-input hashes verified; zero desyncs or missed comparisons |
| Named native lifecycle | All four phases passed: ordinary and save/load recordings, each replayed headlessly and graphically through EOF with hashes verified |
| Harness regression tests | 17 lifecycle and 13 quality-suite tests passed |
| Formatting and whitespace | Passed |

The library counts overlap across feature configurations and must not be summed
as distinct tests. Existing ignored corpus/platform cases remain unclaimed.

Retained native artifacts and runtime evidence:
`/tmp/robin-boundaries-final.0XWeSD/`, including adjacent core assets/mods.

| Artifact | SHA-256 |
| --- | --- |
| `robin` (release features, dev build profile; runtime-tested) | `9013b27df59a9f485475f1de4a0e7f3a8a8a678e510b22e4cecf398cc08c9d5d` |
| `robin-default` (default features, dev build profile) | `b74ef8932741cf4cfae6182df3f9935a8f6c782cfa89ea8206d0175435a9cca7` |
| Browser test module | `eba30ebd80d0a3a3cef82b94d0046ea24af950d35b9c8f471ac5e98ecf0ceacf` |

Native machine-readable summaries are under `multiplayer-headless/`,
`multiplayer-graphical/` and `native-lifecycle/` in that directory. Browser
evidence is `/tmp/robin-lifecycle-gate-048gdjv_/summary.json`; Chrome and driver
were 152.0.7977.64 and the wasm-bindgen test runner was 0.2.127. Browser/native
gate source fingerprints remained unchanged from start to finish.

### Handoff and limits

The completed branches are retained in
`/tmp/robin-boundaries-final.0XWeSD/refactor-boundaries.bundle`, with prerequisite
`930eebbbc2c69e63d2b7858de81758db83bca5ee`, before removing only these five
worktrees/branches: `refactor-boundaries-integration`, `refactor-session-protocol`,
`refactor-gpu-banks`, `refactor-menu-banks`, and `refactor-mission-stages`.
The bundle preserves commits, not ignored build artifacts; the tested binaries
and browser module are retained separately. `/tmp` evidence is local, not durable
backup: copy it before system cleanup if needed.

No wire/save format or exact-build replay admission changes were introduced.
Runtime acceptance used the Leicester demo and disabled sound; it does not
establish full-game corpus parity, audible output, process-restart identity
durability, or remote/browser multiplayer end-to-end behavior. Native GPU
execution used Vulkan, not a full browser game/rendering run. No push, deployment,
golden update or edits to real player data were performed.

Remaining cleanup is intentionally bounded: legacy generated preview IDs and
integer subpicture cache keys remain outside the typed bank migration; mission
preparation can be decomposed further after the concurrent performance changes
land. A server actor/mailbox rewrite should require throughput evidence rather
than being inferred from this authorization cleanup.
