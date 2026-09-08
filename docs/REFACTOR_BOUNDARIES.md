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

TODO: record implementation commits, independent review findings, combined
acceptance, exact binary/browser provenance, recovery bundle and remaining limits.
