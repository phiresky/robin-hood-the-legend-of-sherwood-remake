"""Reopen approved shed receivers and bind protected source/filled appearance."""
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'),
                str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from refinement_workspace import _geometry
from render_slots import acquire, release


def main():
    asset = 'croisement02-woodcutters-shed'
    worker = OUT / 'restart2-vegetation/shed-package-v1/assets' / asset
    model = worker / 'model.blend'
    expected = '0683816e38cf125ecb1e56a7dbdaedb3b666880ff0670c188d2e74c0e5b525da'
    assert sha(model) == expected
    old = OUT / 'texture-fill-round-1' / asset / 'experiment-retry-roof/bake-v1/worker.blend'
    old_hash = '5ec1a62d56bca7efdc45e5ca2dff31f15bb3519f6f563fdef13dc63f460aca6b'
    assert sha(old) == old_hash
    bpy.ops.wm.open_mainfile(filepath=str(old))
    protected = {o.name: _geometry(o, protect_appearance=True) for o in bpy.data.objects
                 if o.type == 'MESH' and o.get('asset_group') == asset}
    assert len(protected) == 2
    bpy.ops.wm.open_mainfile(filepath=str(model))
    assert protected == {name: _geometry(bpy.data.objects[name], protect_appearance=True)
                         for name in protected}
    audit_path = worker / 'inspection/saved-model-audit.json'
    audit = json.loads(audit_path.read_text())
    assert audit['model_sha256'] == expected
    name = 'Shed front boards and recessed opening'
    added = next(o for o in audit['objects'] if o['object'] == name)
    others = [o for o in audit['objects'] if o['object'] in protected]
    source = OUT / 'restart2-vegetation/shed-front-v6/front-source.png'
    image_entry = next(i for i in added['used_materials'][0]['images']
                       if i['name'] == 'front-source.png')
    assert sha(source) == image_entry['packed_sha256']
    cfg_path = worker / 'source-masks.json'
    cfg = json.loads(cfg_path.read_text())
    assignment = next(r for r in cfg['projections']['exterior']['assignments']
                      if r.get('asset_group') == asset)
    dest = OUT / 'restart3-shed-texture-constraints-v1'
    dest.mkdir(exist_ok=False)
    image = np.asarray(Image.open(source).convert('RGBA'))
    domain = dest / 'shed-observed-domain.png'
    Image.fromarray(image[:, :, 3]).save(domain)
    guard_path = dest / 'original-appearance-preservation.json'
    write_json(guard_path, dict(status='PASS', model_sha256=expected,
               original_model=str(old), original_model_sha256=old_hash,
               protected_appearance=protected, exact_reopened_comparison=True))
    record = dict(asset_id=asset, worker=str(worker), model_sha256=expected,
        editable_object=name, original_meshes_fully_protected=protected,
        original_materials_and_images=[dict(object=o['object'], materials=o['used_materials']) for o in others],
        native_material=added['used_materials'][0]['name'], native_material_index=0,
        native_image=str(source), native_image_sha256=sha(source),
        native_image_policy='Keep entire packed front-source.png exact, including alpha. Native RGB feeds Emission Color through the alpha-controlled mix; original filled meshes are wholly protected.',
        protected_domain=str(domain), protected_domain_sha256=sha(domain),
        native_domain_pixels=int((image[:, :, 3] > 0).sum()), native_masks=[127],
        excluded_masks=sorted(set([110] + assignment.get('exclude_mask_indices', []))),
        native_uv_map='Native front projection',
        native_uv_formula=['worldX/1792', '1-(-worldY*sin(35deg)-worldZ*cos(35deg))/1152'],
        editable_appearance=['Material0 own-native-board-supplement fallback only where front-source alpha is zero',
                             'Material1 hidden board edges and rear, inferred dark wood'],
        inferred_uv_map='Inferred board grain', inferred_material_index=1,
        inferred_material=added['used_materials'][1]['name'],
        framing='Use complete saved-material eight views; retain original approved roof and side appearance in the guide.',
        audit=str(audit_path), audit_sha256=sha(audit_path),
        preservation_evidence=str(guard_path), preservation_evidence_sha256=sha(guard_path),
        source_mask_manifest=str(cfg_path), source_mask_manifest_sha256=sha(cfg_path))
    write_json(dest / 'receivers.json', dict(status='Exact approved receiver constraints; no model or source ownership changes', records=[record]))
    assert sha(model) == expected and sha(old) == old_hash
    print(dest / 'receivers.json')


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
