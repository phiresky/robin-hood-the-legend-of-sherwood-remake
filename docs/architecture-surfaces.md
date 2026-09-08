# Renderer-local uploaded surface ownership

`SurfaceHandle` is now a borrowed, renderer-minted reference. Its process-local
renderer identity is skipped by serde, so diagnostic JSON cannot recreate live
GPU authority. `SurfaceTarget::Screen` is distinct from an uploaded surface;
legacy IDs 0 and 1 resolve to it only at the compatibility boundary.

`OwnedSurface` is a non-Clone retirement token. The renderer tracks adopted IDs,
rejects second owners, and prevents integer-based deletion from bypassing an
owner. Retirement validates the originating renderer before changing residency.
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
Execution results will be recorded after the builds finish.
