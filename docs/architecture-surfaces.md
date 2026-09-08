# Renderer-local uploaded surface ownership

`SurfaceHandle` is now a borrowed, renderer-minted reference. Its process-local
renderer identity is skipped by serde, so diagnostic JSON cannot recreate live
GPU authority. `SurfaceTarget::Screen` is distinct from an uploaded surface;
legacy IDs 0 and 1 resolve to it only at the compatibility boundary.

`OwnedSurface` is a non-Clone retirement token. The renderer tracks adopted IDs,
rejects second owners, and prevents integer-based deletion from bypassing an
owner. Retirement validates the originating renderer before changing residency.
Fallible adoption and compatibility deletion return typed errors. Failed typed
retirement returns the original token, allowing recovery with the correct
renderer; mission map replacement and retirement likewise offer non-mutating
error paths. GPU negative tests use these paths, without relying on unwinding.
Borrowed handles support validated dimensions and drawing; queued draws retain
their cloned GPU bindings after retirement.

Mission minimap/marker banks and save/load menu thumbnails now retain ownership
tokens. Mission replacement validates the existing renderer and every new bank
upload before retiring previous resources. Sparse frame positions and layout
metadata remain unchanged. Snapshot reset retains mission resources, and owner
diagnostic decoding still yields an empty mission owner.

Compatibility scope: existing integer upload/draw APIs remain for unmigrated UI
callers. Integer IDs alone cannot establish their originating renderer; convert
them at the upload site and retain typed handles/tokens thereafter. TODO: migrate
remaining UI resource creation and drawing at their owner boundaries rather than
changing every historical ID API at once. Tokens require explicit retirement
with the renderer; renderer teardown releases abandoned GPU resources.

Validation: focused mission ownership tests and the named Vulkan GPU contract
gate cover sparse/repeated retirement, replacement, duplicate ownership, screen
aliases, cross-renderer same-number IDs, diagnostic decoding, and queued draws.
Results on source `8147ffa73`:

- `cargo test --locked -p robin_rs --lib mission_render_resources`: 7 passed.
- `cargo test --locked -p robin_rs --lib surface_diagnostics_never_restore_renderer_authority`: 1 passed.
- `bash scripts/check-quality.sh gpu`: 1 Vulkan multipass/ownership contract
  passed in 2.38 seconds; isolated runtime directory
  `/tmp/architecture-surfaces-gpu.kD9AbL`.
- `cargo fmt --all` and `git diff --check`: passed.

Build/test commands used `CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2`; initial cold
test compilation took 20m46s. Existing unrelated compiler warnings were retained.
The integration coordinator owns the final combined executable build and tests.
