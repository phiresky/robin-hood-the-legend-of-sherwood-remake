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
