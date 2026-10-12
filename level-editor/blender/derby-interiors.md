# Derby interior appearance repair

The West Tower (`patch-001`), East Hall (`patch-002`), and Upper Gatehouse
(`patch-003`) use asset-local `appearance-1` bindings, mapped independently by
placements in `scenes/derby.rhlos-map.json`. Main Hall (`patch-000`) retains its
existing binding; its subsequent texture repair is documented below. Gameplay
masks, sight obstacles, and doors are
unchanged: these are editor appearance repairs.

`derby_interior_states.py` operates on the published lossless models. It retains
all covered binary payloads, accessors, meshes, materials, images, transforms,
and canonical part identities. Revealed surfaces are appended under the same
parts. It rejects an input that already contains reveal bindings.

- West Tower: cut the room opening from facade part 132, retaining the existing
  room floor, rear wall, curved cut-wall crown, and surrounding tower.
- East Hall: cut the patch silhouette from facade parts 183/185/188/189/191/192/
  211/212. Retain the upper floor, roof, battlements, lower masonry, and furniture.
- Upper Gatehouse: switch off only the two authored removable chamber facade
  components. Keep their separate parapets and retained wall components.

Cutting interpolates vertex attributes from the original triangles; it does not
stretch existing UVs. Alpha contours are simplified within 0.5 source pixels.
Edge-on triangles are clipped as intervals so camera rotation does not expose an
uncut wall. This is a source-camera cutaway, not a change to collision geometry.

Revealed artwork is projected onto the actual first-hit surfaces inside the
native patch alpha. Bridge geometry remains an occluder but receives no room
artwork. Connected neutral texture placeholders of at least 64 texels receive
small stone, wood, or cloth source swatches. Those unobserved fills have ownership
alpha zero; source-observed texels have alpha one. These fallbacks remove gray
faces without representing inferred surfaces as observations. They remain
approximate, especially from oblique views; no new texture generation was used.

## Reproduction and validation

Use a Python environment with `derby-interiors-requirements.txt` installed.
The source must contain the pre-repair `model.glb` and `asset.json` for all three
assets; publication backups preserve those inputs. The reference directory must
contain the retained game-data `layers.json`, alpha masks, and `revealed.png`.
Use a fresh output directory for each revision:

```sh
python3 level-editor/blender/derby_interior_states.py SOURCE REFERENCE STATES
python3 level-editor/blender/derby_publish_interiors.py stage level-editor/library STATES PUBLICATION
blender --background --factory-startup --threads 2 --python-exit-code 1 \
  --python level-editor/refinement/blender/lossy_assets.py -- refresh \
  --root PUBLICATION/map-assets/3d-assets --work PUBLICATION/derivatives \
  --assets derby-keep-west-tower derby-east-hall derby-upper-gatehouse
python3 level-editor/refinement/prepare_publication_browser.py \
  PUBLICATION PUBLICATION/scope.json PUBLICATION/browser/config.json \
  --map derby --document PUBLICATION/derby.rhlos-map.json
node level-editor/refinement/browser/verify_publication.mjs \
  PUBLICATION/browser/config.json http://127.0.0.1:5181
```

Inspect covered/revealed game-camera and oblique renders before installation:

```sh
python3 level-editor/blender/derby_publish_interiors.py apply level-editor/library PUBLICATION
```

Installation requires the full browser audit, checks the exact audited hashes,
locks the library, backs up affected files, and restores them if validation fails.
Refreshed lossy models and covered previews accompany the lossless state models;
their receipts bind the exact new source bytes. Unrelated library assets are preserved.

Regression checks:

```sh
python3 -m unittest discover -s level-editor/blender -p test_derby_interior_states.py
cd level-editor
pnpm --filter pipeline exec node --test ../app/src/patch-display.test.ts ../shared/src/patch-bindings.test.ts
```

Current repair evidence is under `work/needs-review/derby-interiors-fix/`: `states-v4/` records
input/output hashes and covered-resource invariance; `covered-comparison.json`
records zero changed pixels in all three covered renders; `review.png` compares
the room references and restored appearances. The publication browser audit
requires all four interior patches and checks the actual editor controls,
selection, insertion, save, and reload.

The repair is installed in the canonical library and Derby scene. The full browser
audit passed with all four reveal controls, 40 map groups, 271 selectable parts,
40 standalone insertions, duplicate/undo/redo, save, and full-page reload. Five
cutaway regression tests and eight patch-display/binding tests pass. The local
`publication/installed.json` records all 19 installed file hashes and the audit
result hash; `publication/backup/` retains the prior files.

Main Hall’s remaining gray revealed surfaces are repaired separately; see
[the Main Hall texture notes](derby-main-hall-texture.md).
