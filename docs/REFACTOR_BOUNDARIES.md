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
- Mission loading now consumes explicit preparation, engine-construction and
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

TODO: record final combined acceptance, exact binary/browser provenance,
recovery bundle and remaining limits before merging into main.
