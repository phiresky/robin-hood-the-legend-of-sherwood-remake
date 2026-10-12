# Lincoln refinement

Implements `level-editor/refinement/PROCEDURE.md` for Lincoln. Run from the repository
root; evidence lives in `level-editor/work/needs-review/lincoln-refinement/` (resume from `RESUME.json`).

1. `source_states.py`: covered/revealed artwork, native masks and patch-state frames → `source-states/`.
2. `setup_scene.py` (Blender) freezes the volume export into `baseline/lincoln-baseline.blend`
   and writes the inventory. `--annotate-states` records patch membership; `--illustrate` renders crops.
   All 466 native obstacles are present. The exporter's `terrace-N` names are normalized to `building-N`.
3. `grouping/build_catalog.py` + `validate_review.py` (Blender): reviewed canonical catalog, 96 groups.
4. `calibrate_source_sun.py`: artwork shadow landmarks → `lighting-calibration/map-lighting.json`.
   Packets pass it explicitly (`prepare_assets.py --lighting`); Derby's default is never used.
5. `mask-review/build_manifest.py`: reviewed exterior receivers (`source-masks-v1.json`).
6. `group_scene.py` (Blender) applies the catalog → `grouped/lincoln-grouped-v1.blend`.
   It renames `Lincoln …` scene/collections to `lincoln …`, because shared helpers derive
   `<catalog map> Working`.
7. `prepare_assets.py` (Blender) creates frozen per-asset workspaces in `round-1/assets/`.
8. Lane workers follow `work/needs-review/lincoln-refinement/WORKER_BRIEF.md` with `refine_<lane>.py` recipes.
9. `build_gallery.py` collects packets into the pending-only review gallery. It requires a
   hash-bound `source-coverage-audit.json` for every ready card.

10. Publication (staging never touches the live library):
    - `publish_stage.py` (Blender) takes a hash-bound plan (e.g. `publication-1/stage-v1.plan.json`):
      grouped baseline plus the approved models that differ from it. It writes `worker.blend`, the
      staged catalog and `integration.json`.
    - `global_reproject.py apply` (Blender) is a publication-level material fix on the staged worker:
      every still-unknown texel of a first-hit source-visible surface receives its source RGB (masks
      are not applied; grazing faces below `--grazing-cosine`/`projection_min_cosine` are reset to
      unknown). `global_reproject.py audit` renders any worker from the source camera and counts gray
      pixels; `verify_global_reprojection.py` proves only unknown texels changed. Fill masks in
      `<stage>/global-reprojection/` tell generated-texture bakes which texels are now source-known.
      `stage_combined_worker.py` stages a texture-combined worker on top.
    - `texture_combine.py` fills remaining neutral texels from ready generated sheets;
      `texture_unseen_fill.py` generates and fills texels no source camera sees (targets in
      `textures/unseen/targets.json`, approvals in `textures/unseen/decisions.json`);
      `revealed_state_bake.py` bakes approved revealed states onto state-only objects.
      Each output is staged with `stage_combined_worker.py`. See the procedure's
      publication-level texture completion and revealed-state sections.
    - A plan may point `approvals` at a frozen `approvals-snapshot.json` in the stage when newer
      approvals must wait (see `publication-2/stage-v7`).
    - `verify_publication_scene.py` checks every approved model against the staged worker (after a
      global reprojection only `materials` may differ; run it on the unmodified stage-in as well).
    - `publish_export.py` exports the full map (component splits become
      `building-NNN--component-<name>` parts), the standalone assets and `publication-metadata.json`.
      Freeze its tooling into the publication directory with `freeze_tooling.py --output`.
    - Then run `verify_publication_assets.py`, `verify_staged_handoffs.py` (with
      `effective-plan.json`), `pipeline/src/prepare-publication-document.ts` (first
      `lincoln.rhlos-map.json`), `prepare_publication_browser.py --document` with
      `browser/verify_publication.mjs`, and finally `promote_staged_publication.py`.
    - A texture republish is a new plan whose imports name the approved baked workers.

`freeze_tooling.py` pins shared helpers in `tooling/<id>`. `render_slots.py` takes a lease
from the shared machine-wide pool of four slots (same lock files as `refinement/render_slots.py`).

Map-wide finding: native obstacles extrude from z = 0, but the castle plateau is at native
z = 220. Every castle object must be reseated on its real ground contact.
