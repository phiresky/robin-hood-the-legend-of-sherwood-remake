# Hidden-archer foliage screen qualification

CPU-only source and model evidence for four distinct mission foliage screens.
This is a reconstruction handoff, not approved geometry or a publication.

Run `qualify.py`, `model_evidence.py`, then `reconstruct.py` with Python, NumPy
and Pillow. Run `python3 -m unittest discover -s <this-directory> -p 'test_*.py'`.
Outputs go to `work/croisement02-refinement/mission-hidden-archer-physical-oct12-v1`.

The qualification pins the current map, native binding packet, source artwork,
and candidate models/descriptors. The geometry diagnostic applies the complete
GLTF node hierarchy and actual map placement, projects triangle footprints into
the source camera, and pins external geometry buffers. It deliberately includes
back faces and alpha-discarded faces: cyan coverage is only conservative context,
never proof of first-hit ownership or attachment.

The reconstruction handoff stores exact per-column opaque pixel runs for all
16 images and identifies unchanged contact context. A new screen changes shape;
a texture-only replacement or a reused tree crown does not establish parity.
Depth, unseen thickness, back surfaces and attachment construction are unresolved
inferences. Variant02 needs complete inferred off-map geometry with native display
clipping kept separate. Approval of climbing variant05 does not cover these four.

The source timing remains three transition images held for two eligible updates
each, followed by terminal background integration. Callback actors and latches
are independently owned. No Tac19 occupied-net/capture state is introduced.

## Private geometry derivatives

`probe_contacts.py` queues alpha-aware native receiver rays in the shared two-slot
FIFO pool. `build_models.py` builds a new immutable rounded variant02 derivative;
the initial all-four construction recipe is preserved beside `geometry-v1`.
Every profile file contains all four authored poses. Source faces retain exact
nearest-sampled pixels; unknown closed backs and edges are gray. The upper border
of variant02 is completed with inferred, separately marked geometry.

Saved files are reopened for textured and solid eight-view sheets for both
endpoints. Original camera is always first. Closed-edge counts, native silhouette
center checks and actual stored-UV samples are recorded for every pose. These
checks do not establish coherent attachments or whole-scene source visibility.

`contact_models.py` clips installed receiver triangles against each opaque pixel's
physical prism and evaluates bilinear texture alpha over the clipped polygon.
It is an endpoint construction-volume audit, not continuous motion approval.
Its first version binds the original depth proposal and original top continuation.
Any revised geometry requires a separately bound contact result.

## Approved profiles 03 and 04: texture completion

The grouped `next23-review-hub-oct12-v1` decision approves the exact profile03
four-pose model with eight short supports and profile04 with eleven supports.
Their frozen geometry cards remain unchanged. The texture pipeline is:

1. `prepare_approved03_04_textures_v1.py` derives conservative editable first-hit
   masks using the existing approved camera pixels. It preserves native material
   slots and all geometry/UVs. No geometry rerender is needed.
2. `bind_approved03_04_textures_v1.py` binds that decision to eight pose packets,
   using the shared texture preparation contract and both permitted Leicester
   material examples. The transport pads the bottom to 1024×640; it does not
   rescale the 1024×512 content.
3. The output `generate-sequential.sh` runs the authorized repository texture
   API with one request at a time, outside the render pool. Raw outputs are
   archived; protected compositing restores every known/background pixel.
4. After visual review, `bake_approved03_04_textures_v1.py -- <profile>-<phase>`
   projects generated appearance only onto inferred faces. Each polygon needs
   a best-facing camera witness that sees that polygon as the first hit and
   marks it editable. Unobserved polygons remain enumerated, not silently filled
   from unrelated surfaces. Original source UVs/materials are retained.
5. Run `review_approved03_04_textures_v1.py -- <profile>-<phase>` in a new guarded
   process to inspect all eight actual saved-material views. Geometry, original
   source materials and UV hashes must still match. Final texture approval and
   runtime integration remain separate requirements.

All background invocations use the shared FIFO resource guard before loading a
model. Save and exit before scheduling the saved-model review.

## Canopy05 source-order qualification

`compose_actual_canopy05_v1.py` compares actual sampled physical RGBA with native
loop/patch compositions. The complete physical owner contains both animated
crowns and static wood, so its extra alpha cannot be classified from native loop
alpha alone. Keep its approved geometry; distinguish static source contribution
from the moving owner before transferring or suppressing receiver parts.

All Archer01 and Archer02 occurrences are zero-elevation base effects. Their
background-animation routing precedes polyline handling, so elevated canopy05
is drawn afterward even when a patch display polyline is empty. The previous
empty-polyline sentinel interpretation is superseded. A patch-foreground image
is retained only as a counterfactual diagnostic. Static wood and static foliage
must not gain moving-owner draw order merely because they share an export.
