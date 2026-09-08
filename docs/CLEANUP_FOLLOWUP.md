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

TODO: record reviewed implementation commits, combined explicit tests, native
multiplayer and rendering lifecycle evidence, actual browser results, and
remaining limits after acceptance completes.
