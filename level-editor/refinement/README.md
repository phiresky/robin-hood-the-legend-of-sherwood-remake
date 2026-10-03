# Shared map refinement tooling

Start with [PROCEDURE.md](PROCEDURE.md): it is the canonical workflow. This file
is the index of where the Python lives and which entry points to run.

## Layout

| Path | Contents |
|---|---|
| `refinement/*.py` | Pure-Python tooling: approvals, texture packets, staging, promotion, library install, `render_slots`, `run_tests` |
| `refinement/blender/*.py` | Everything that runs inside Blender (bpy): inventory, grouping, workspaces, review renders, projection, bakes, export, verification, plus their tests |
| `refinement/recipes/<name>/` | Map-specific recipes that reuse the shared tooling (Derby shelter texture recovery) |
| `refinement/plans/` | JSON staging plans for `blender/stage_approved_batch.py` |
| `refinement/browser/` | Browser audit of staged publications (`verify_publication.mjs`) |
| `../blender/<map>/` | Per-map pipeline scripts and recipes (Leicester, Lincoln, Nottingham, Sherwood); Derby recipes sit directly in `../blender/` |
| `../pipeline/src/refinement/generate-textures.ts` | Two-image texture generation driver (Node dependencies) |

Other docs: [reprojection.md](reprojection.md) (source reprojection and its limits),
[refinement-workflow.md](refinement-workflow.md) (isolated worker workspaces),
[CORNER_EDITOR.md](CORNER_EDITOR.md), [../docs/library-format.md](../docs/library-format.md),
and each map's `../blender/<map>/README.md`.

## Entry points

Per-map pipeline, in order (each map has its own copy under `../blender/<map>/`):
`setup_scene` → `group_scene` → `source_states` → `calibrate_source_sun` →
`prepare_assets` → lane recipes (`refine_*`, run in Blender workers) →
`build_gallery` → explicit approval → texture packets and bakes →
`publish_stage` → `publish_export` → the shared verification and promotion below.

| Step | Script | Notes |
|---|---|---|
| Inventory / grouping | `blender/refinement_inventory.py`, `blender/group_assets.py` | [PROCEDURE §13](PROCEDURE.md#13-reusable-scripts) |
| Occlusion-depth inspection | `masks_to_depth.py` | Inspect every mask layer; optional `--mask-ids` writes a labeled preview. [PROCEDURE §4](PROCEDURE.md#4-use-masks-and-patches-as-ownership-authority) |
| Scene import, camera renders | `blender/setup_map.py`, `blender/render_views.py`, `blender/inspect_asset.py` | Derby-era helpers, still imported by map setup |
| Isolated workers | `blender/refinement_workspace.py`, `blender/run_worker.py` | [refinement-workflow.md](refinement-workflow.md) |
| Review renders and galleries | `blender/refinement_review.py`, `blender/render_multiview_asset.py`, `blender/build_review_gallery.py` | |
| Source projection | `blender/source_projection_bake.py`, `blender/reproject_map.py`, `blender/projection_regions.py`, `blender/occlusion_constraints.py`, `blender/interior_layers.py` | [reprojection.md](reprojection.md) |
| Approval records | `record_approval.py`, `texture_decisions.py`, `publication_authorization.py` | |
| Texture packets | `prepare_texture_packet.py`, `prepare_planar_texture_packet.py`, `prepare_uv_atlas_texture_packet.py` | |
| Texture bakes | `blender/bake_reviewed_asset.py`, `blender/bake_approved_packets.py`, `blender/bake_planar_texture.py`, `blender/bake_uv_atlas_texture.py`, `blender/project_reviewed_texture.py` | |
| Texture gallery | `build_texture_gallery.py` | |
| Staging | `blender/stage_approved_batch.py`, `blender/stage_reviewed_publication.py`, `blender/bind_patch_material_states.py`, `blender/export_editor.py` | |
| Verification | `blender/verify_staged_handoffs.py`, `blender/verify_publication_assets.py`, `blender/verify_staged_patch_state.py`, `blender/render_staged_patch_state.py`, `publication_preflight.py` | |
| Promotion | `prepare_publication_browser.py` → `browser/verify_publication.mjs` → `promote_staged_publication.py` (`--waive-browser-check "<reason>"` only on the user's explicit decision), `apply_canonical_library.py` | [PROCEDURE](PROCEDURE.md#map-publication-format) |
| Separate state endpoints | `bundle_publication_states.py`, `promote_state_bundles.py` (not for state objects baked into a model) | [PROCEDURE §10](PROCEDURE.md#revealed-states-inside-per-asset-models) |
| Browser derivatives | `blender/lossy_assets.py` (`refresh`, `library`, `rollback`), `../pipeline/src/preview-model.ts` | [PROCEDURE](PROCEDURE.md#browser-derivatives-lossy-models-and-previews) |
| Publication-level texture completion | `../blender/lincoln/global_reproject.py`, `texture_combine.py`, `texture_unseen_fill.py`, `stage_combined_worker.py`, `revealed_state_bake.py` (Lincoln recipes) | [PROCEDURE](PROCEDURE.md#publication-level-texture-completion) |
| Corner editor | `prepare_corner_editor.py`, `corner_editor.py`, `audit_corner_constraints.py` | [CORNER_EDITOR.md](CORNER_EDITOR.md) |

Every script has a module docstring; `--help` lists its arguments.

## Conventions

- **Imports.** Blender scripts put their own directory on `sys.path` and import
  shared helpers by bare module name. Map recipes that run in workers call
  `freeze_tooling.select_tooling()` first: it pins a content-addressed copy of
  `refinement/blender/*.py` (and the Derby recipes in `../blender/`) from
  `work/<map>-refinement/tooling/<id>/`, so later edits to live helpers never
  change an existing worker's implementation. Pure-Python modules in
  `refinement/` are not part of those snapshots.
- **Render slots.** `render_slots.acquire()` (in `refinement/`) takes one of
  four machine-wide slots in `work/lincoln-refinement/render-slots/` (FIFO); every
  map, worker and lossy derivation uses that one pool. Acquire before loading
  large scenes.
- **Recipe provenance.** Gallery builders re-hash the `candidate.json` `"recipe"`
  path on every rebuild. New packets should call
  `evidence_io.record_recipe(workspace, __file__)` and store its workspace-relative
  `recipe`, so the live script can later change or be deleted. Never edit a recipe
  in place once its hash is bound in evidence; add a new one. Old recipes are
  deleted rather than archived; git history (tag `python-cleanup-base`) and the
  recorded `recipe_sha256` values keep them recoverable.
- **Helpers.** `blender/evidence_io.py` has `sha`, `digest`, `read_json`,
  `write_json` and `record_recipe`; use it instead of adding another copy.
- **Tests.** `test_*.py` sits next to the module it tests. Run everything with
  `python3 level-editor/refinement/run_tests.py` (plain tests in python3, tests
  that import bpy in `blender --background --factory-startup`); `-k <text>`
  filters, `--no-blender` skips Blender tests, `--selftests` adds the
  `*_selftest.py` integration runs.

## Notes on individual tools

Collectors that produce a gallery ownership report can call
`record_gallery_decision(gallery_path, records_path, asset_id, decision, exact_text)`
from `record_approval.py`. It keeps decisions separate from regenerated candidate
manifests, binds the displayed model and render packet, archives approved files,
and retains previous decisions when a user requests another revision. Approved
items hidden from the pending page remain addressable through gallery history.
- `../pipeline/src/refinement/generate-textures.ts`: shared two-image generation
  driver; kept inside the pipeline package for its Node dependencies.

The former `level-editor/blender/*.py` compatibility forwarders and low-level
helpers were folded into `refinement/blender/` on 2026-09-26; commands saved
before then must use the new path. Map-specific recipes and output packets stay
with their map; moving their coordinates would not make them generic.

```bash
python3 level-editor/refinement/blender/build_review_gallery.py \
  <review-manifest.json> <gallery-dir> --pending-only --map-name <map>
python3 level-editor/refinement/record_approval.py \
  <review-manifest.json> <asset-id> --decision '<actual user approval and scope>'
```

The gallery uses stable asset IDs and content hashes in image filenames, and
stable asset IDs as HTML anchors. It archives prior evidence before rebuilding.

For a batch of approved geometry/texture handoffs, use
`blender/stage_approved_batch.py` with a JSON plan (see
`plans/derby-approved-buildings-20260923.json` for the schema). Paths are resolved
against the repository root, source blends are hashed, and imports are restricted
to each catalog group's canonical parts. `source_asset_id` explicitly handles a
worker retaining an older parent group; it never imports the entire old group.
The script stages a JSON map and one local asset catalog without updating the live
library. Map placements and palette entries share the same models and payloads;
mission bindings and source-map origins live in the map JSON. The final map export
also converts selected standalone endpoints into the shared catalog. See
[the library format](../docs/library-format.md) and the
[publication procedure](PROCEDURE.md#map-publication-format).

Run `blender/verify_staged_handoffs.py` in Blender against the resulting resolved
`.plan.json` before promotion. It compares untouched meshes exactly and checks
each imported handoff's world geometry, UVs, material graphs and packed image
bytes. World-coordinate drift below 0.001 units is allowed for float32 parenting
roundoff and is reported per asset; topology and appearance must match exactly.

When one surface needs different materials in covered and revealed states,
`blender/bind_patch_material_states.py` binds complete reviewed face assignments
and exact cover visibility from two workers. The exporter retains both alternatives
inside one canonical part; the editor's patch preview displays only the selected
alternative. `blender/verify_staged_patch_state.py` verifies the revealed state
against its source worker, and `blender/render_staged_patch_state.py` reproduces
its exact saved cameras. Inspect these images and check that they contain the
asset before claiming zero-pixel preservation: two empty renders can also match.

Texture bakes can optionally pass a raw generated reference after the texel-density
argument to `blender/bake_reviewed_asset.py`. The preserved image remains selected;
raw RGB only calibrates inferred-color gains. See the bake section in
[PROCEDURE.md](PROCEDURE.md) for evidence and review requirements.

For a single approved planar atlas, `prepare_planar_texture_packet.py` derives
coverage from audited UV triangles without resizing the atlas. The paired
`blender/bake_planar_texture.py` preserves UVs and geometry and retains all eight
actual review views. See the planar exception in the procedure.

Record explicit texture feedback separately from geometry approval:

```sh
python3 level-editor/refinement/texture_decisions.py <texture-gallery> <texture-review>/decisions.json <feedback.txt>
```

The recorder verifies every displayed revision and its current baked model,
images and reports before recording the batch, then archives the selected
evidence. Rebuilding with `build_texture_gallery.py` hides matching approved
textures and retains the full candidate manifest. Changed evidence needs a new
decision; recording approval does not publish the asset.
