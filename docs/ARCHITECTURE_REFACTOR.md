# Architecture follow-up implementation

Base: public-release tree `cc36f8d75`. This pass implements the nine approved
refactoring recommendations in ten isolated implementation worktrees. An
independent read-only audit and the integration lane review cross-cutting
invariants. Concurrent performance worktrees and untracked original sources
are outside this pass.

## Tracks and acceptance

| Track | Concrete boundary | Required evidence |
| --- | --- | --- |
| Multiplayer lifetime | Campaign-owned identity and mission continuation; no process-global slot | Independent owners, failed replacement, reconnect and shutdown |
| Multiplayer protocol | Shared transport-independent framing and client transitions | Native/browser-equivalent event traces; unchanged wire/security checks |
| Engine authority | Guarded posture mutations and one cohesive movement/order capability | Exact lifecycle, callback order, persistence and engine tests |
| Prepared inputs | Grouped level resources and validated deterministic audio publication | Existing optional/required resource policy, load/replay/projection contracts |
| Presentation authority | Exact host/frontend borrows at render/audio boundaries | Boundary regressions; fixed-tick/display-refresh and screenshot behavior |
| Rendering ownership | Renderer-local handles and another explicit UI upload owner | Wrong-renderer rejection, screen aliases, replacement, queued draw survival |
| Cache jobs | Single-flight load outside the owner lock, cancellation and generation publication | Concurrent waiters, invalidation, stale workers, retirement and failure |
| Audio residency | Bounded encoded browser audio retention | Cap/eviction/oversize/in-flight behavior and real-browser tests |
| Content contracts | Shared asset/simulation leaf contracts and measured dependency boundaries | Unchanged formats/hashes; affected asset/engine/consumer checks |
| Acceptance gates | Reproducible audio-enabled WASM execution and native lifecycle checks | Orchestration tests, real browser execution, isolated live/save/load/replay |

## Compatibility constraints

- No deliberate gameplay, RNG, callback-order, save schema or wire changes.
- Preserve exact-build replay admission; do not rewrite reference recordings.
- Preserve optional artwork/speech behavior and existing strict ranked admission.
- Process/GPU authority must not be revived by diagnostic deserialization.
- The engine-capability and content-dependency work uses bounded cohesive slices,
  not a whole-engine/ECS rewrite or wholesale relocation of gameplay types.
- Run explicit affected package suites, format and relevant target/features.
  Builds and bounded runtime checks are separate; no Clippy cleanup is mixed in.

## Integration record

TODO: append implementation checkpoints, code-review findings, test outcomes,
runtime evidence and remaining limitations as each track is integrated.
