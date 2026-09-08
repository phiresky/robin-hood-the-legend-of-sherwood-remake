# Typed GPU sprite banks

Mission map, minimap corner, dot and ground-mark accessors now lend
`SurfaceHandle` values instead of renderer-local integers. Their consumers retain
those handles through the final draw. Ordinary, alpha and shadow draws check both
renderer identity and upload lifetime before queuing work; their existing pixel
rounding, clipping, transparency and shadow implementations are unchanged.

The upload/adoption boundary still accepts freshly allocated legacy IDs from the
existing image loaders. Ownership never crosses this boundary as an integer in
the migrated draw path. Map and sprite-bank replacement preflight all candidates
before claiming any upload or retiring the previous bank. Fallible replacement
APIs make duplicate, missing, already-owned and foreign-renderer errors testable
without depending on panic unwinding. Optional sparse frame slots and the
minimap corner's existing fallback behavior are preserved.

GPU acceptance exercises same-number foreign handles, stale borrowed handles,
alpha/shadow rejection before queue mutation, duplicate/reused candidate rejection,
screen aliases, failed-prefix ownership rollback, retained corner dimensions,
replacement retirement and queued bindings surviving retirement. Pure tests retain
sparse-frame, idempotent retirement and diagnostic-deserialization contracts.
Four previous synthetic panic tests are replaced by live-GPU `Result` assertions.

The menu-bank continuation is described in `REFACTOR_MENU_BANKS.md`.

Focused validation at `8846f1805`: `cargo test --locked -p robin_rs --lib
mission_render_resources` passed all three pure ownership tests. This compiles
the default client but does not execute the explicitly opted-in GPU contract.

Final combined source `09a2438b1`: full default-client tests passed (1,569 library,
33 integration, seven doctests), as did the named Vulkan gate, default binary
build and tools/projection-export examples check. See
[combined acceptance](REFACTOR_BOUNDARIES.md) for provenance and runtime evidence.
