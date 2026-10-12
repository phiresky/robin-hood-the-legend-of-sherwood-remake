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
