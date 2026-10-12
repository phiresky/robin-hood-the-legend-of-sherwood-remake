# East Hall recovery handoff

The current source-alignment candidate is `next-zigzag-v3/band-review/model.blend`.
Its authority projection and 48 degree sunlight sheets are beside it in `modified/`.
The report `next-zigzag-v3/review.md` embeds 33 existing images inside its own `images/` directory.

This is ready for visual geometry review, not yet approved for publication or texture synthesis. The east corner and southwest parapet have zero boundary/nonmanifold edges. The rear transition and both cut ends are now closed. Its 43 remaining boundary segments match the baseline's 37 boundary segments geometrically; the difference in count comes from splitting existing edges at the join plane. `boundary-inheritance.json` records that every remaining boundary lies within 0.002 world units of a baseline boundary. Lower facades remain intact, and all 768 protected meshes are unchanged.

The 118 recorded source points distinguish observed corners from inferred hidden caps. Foreground residuals use nearest projected mesh edges, without visibility filtering, and are not independent evidence that the artist's intended corners were identified correctly. Rear and stair residuals use the rendered upper silhouette. The stair/roof junction still has a 5.83 pixel residual.

Recovery sequence:

1. Load `next-zigzag-v3/front-runs-trial.blend`, the newer foreground weld/T-junction correction saved after the earlier packets.
2. Execute `rebuild_rear_visible.py`, then `refine_stair_turret_trace.py` in Blender.
3. Execute `repair_trace_seams.py` on `traced-runs-candidate.blend`. This saves `repaired-candidate.blend`; it only welds serialization-scale seams and collinear edge intersections.
4. Execute `inspect_planar_join.py` on `repaired-candidate.blend` to extract actual lower/upper contours. Run ordinary Python `build_planar_join.py` (requires Shapely; this workspace has it in `python-deps/`). It computes the exposed planar difference, restricting the rear correction to the edited x=1359..1567 strip.
5. Execute `apply_planar_join.py` on `repaired-candidate.blend`. It installs the join, removes the redundant southwest internal separator, and closes the two explicit rear cut-end loops. The result is `joined-candidate.blend`.
6. Execute `inspect_planar_join.py -- 365.01` on `joined-candidate.blend`, run ordinary Python `build_rear_band.py`, then execute `apply_rear_band.py` on `joined-candidate.blend`. This supports the previously floating source-fitted rear floor with a shallow band, producing `band-candidate.blend`.
7. Execute `validate_traced_candidate.py -- <absolute band-candidate.blend path>`, `validate_join_boundaries.py -- <absolute band-candidate.blend path>`, and `validate_lower_surfaces.py` to verify the actual candidate against the casement baseline.
8. Execute `reproject_front_trial.py -- band-review` on `band-candidate.blend`. The output directory must not already exist. This validates the accepted native-mask checkpoint, reapplies layered ownership projection and writes model plus eight views.
9. Execute `inspect_walltop_exact.py -- candidate-edges.json` and `render_trace_silhouette.py` on `band-candidate.blend`, then the ordinary Python scripts `draw_front_comparison.py`, `measure_trace_residuals.py` and `build_trace_report.py`.

`cap_upper_seams.py` is an unpromoted experiment. Its `seam-cap-test.blend` closes 14 upper loops and reduces rear boundary edges to 71, but does not close the remaining transition between the source-fitted parapet footprint and retained lower wall footprint. Do not replace the review candidate with that experiment without inspecting and reprojecting it.

The band candidate supersedes the earlier recovery, joined and cap packets. The intermediate joined model closed the seams but moved rear notch-floor silhouette points by 5–6 pixels; it must not be promoted. The supported band restores those observations to their previous 2–3 pixel maximum residual while retaining the closed joins. Its new surfaces bridge the distinct lower/upper footprints without overlapping separator faces. Revealed interior geometry, the concealed stair-facing edge and the 5.83 pixel stair/roof silhouette junction discrepancy remain unchanged and disclosed in the review. All 2,820 tested lower-facade surface samples remain within 0.057 world units of the baseline; the 0.1 world-unit tolerance is below one source pixel and permits subpixel changes from nonplanar polygon triangulation.

Blender MCP was unavailable at localhost:9876 during recovery. All work used deterministic background Blender processes with two threads.
