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

All ten implementation tracks were merged at `43ab1f432`. The sole textual
conflict was the opacity trait: retain the new `robin_content` re-export and
the prepared-attachment documentation. A combined client compile subsequently
caught one assignment still using `assets.spellforge_runtime`; `eccfa0806`
migrates it to `assets.attachments.spellforge_runtime`.

Per-track design and isolated acceptance reports:

- [Multiplayer lifetime](architecture-network-owner.md)
- [Shared admission core](architecture-network-core.md)
- [Engine mutation authority](architecture-engine.md)
- [Prepared resources](architecture-preparation.md)
- [Presentation capabilities](architecture-presentation.md)
- [Renderer ownership](architecture-surfaces.md)
- [Asset jobs](architecture-cache.md)
- [Browser audio budget](architecture-audio-budget.md)
- [Content dependency boundary](architecture-content-types.md)
- [Repeatable lifecycle gates](validation/lifecycle-gates.md)

The independent audit checked original and combined diffs for posture-source
independence, synchronous movement callbacks, projection payload/hash order,
optional audio policy, post-command routing, pacing clocks, transport admission,
job cancellation and renderer retirement. Review also led to transactional
server startup/bridge cleanup, consistent direct mission restart handoffs, and
fallible GPU rejection tests that do not depend on Cranelift unwinding.

### Baseline runtime evidence

Before any source merge, the integration worktree built the release-feature
native binary on `34bb3fb2d` (documentation only over the unchanged base). The
retained installation is `/tmp/robin-architecture-baseline.f9o2Nq`, including
required core assets and mods. Binary SHA256:
`123e7ceb1bba72803c3a22a1b118ae5eebdcdc1845fa3681259d8735b4645448`.

- Isolated headless multiplayer: all ten checks passed, including late-input
  rollback, in-process reconnect, seat preservation and subsequent hash
  agreement; zero desyncs and zero missed comparisons. Evidence:
  `multiplayer-headless/summary.json` under that installation.
- Graphical multiplayer on the same unchanged binary reproduced an existing
  panic after a late-input rollback: `timeline frame committed without a
  matching begin_frame`. The second graphical ingress can invalidate a pending
  capture opened before that ingress. Evidence: `multiplayer-graphical/peer.log`
  and its summary. This is not a regression attributed to these refactors.

### Combined acceptance

At `43ab1f432`, the expanded named `assets` gate passed (default and codec-only
assets/data I/O, leaf contracts and dependency graph assertions). The parity
suite passed 151 unit tests plus its dependency-closure integration test; ranked
verification passed 23 tests with four explicitly ignored operator-data cases.
The quality-suite orchestration tests (13) and lifecycle wrapper tests (12)
also passed after the dispatch hardening.

TODO: record final combined engine/client/browser/GPU and retained native
lifecycle results after the client field correction and graphical frame-boundary
regression are verified. Do not treat isolated branch results as final combined
acceptance.

### Deliberate limits and next slices

- Movement authority is one cohesive sequence-only operation, not a complete
  rewrite of `EngineInner` or an ECS conversion.
- Shared networking covers framing, admission and reconnect validation. Later
  gameplay/ranked state transitions and server seat-map consolidation remain
  separate slices; native/browser unresolved-ranked policy remains distinct.
- Codec-only consumers exclude the engine (105 versus 144 native normal-graph
  packages); default adapters preserve compatibility and still depend on it.
  This is measured dependency reduction, not a claimed build-time speedup.
- Typed GPU ownership covers mission banks and save/load thumbnails. Legacy
  integer APIs remain for unmigrated UI owners; renderer teardown releases
  abandoned resources.
- Encoded browser retention has a conservative 32 MiB ceiling, separate from
  PCM. It does not cap in-flight buffers or total JS memory; representative
  traces are still needed to tune the default.
- Cache cancellation is cooperative between parsing stages, not preemption of
  an individual read/decode. Browser automation verifies API/lifetime behavior,
  not audible output or full interactive browser gameplay.
