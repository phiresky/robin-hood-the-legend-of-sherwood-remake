"""Assemble a private complete source base from registered, pinned plant workers."""
import json
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,scenery_workspace,reviewed_catalog
from evidence_io import sha,write_json
from render_slots import acquire,release
from stage_review_scene import signature
from refinement_inventory import inventory,validate_catalog


def main():
    directory=OUT/'ground-plant-integration';target=directory/'input.blend'
    if target.exists():raise FileExistsError(target)
    selected=json.loads((directory/'selection.json').read_text())['records']
    inputs=[scenery_workspace(asset) for asset in selected]
    inputs += [scenery_workspace(f'croisement02-shrub-{i:02}') for i in [65,66,81]]
    base=OUT/'ground-plant-candidates/east-grass-v7/input.blend';baseline_hash=sha(base)
    bpy.ops.wm.open_mainfile(filepath=str(base));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];records=[]
    for worker in inputs:
        model=worker/'model.blend';digest=sha(model);audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
        if audit['status']!='PASS' or audit['model_sha256']!=digest:raise ValueError('Stale worker')
        nodes={r['source_node'] for r in audit['objects']};names=[r['object'] for r in audit['objects']]
        for obj in list(collection.all_objects):
            if obj.type=='MESH' and obj.get('source_node') in nodes:bpy.data.objects.remove(obj,do_unlink=True)
        with bpy.data.libraries.load(str(model),link=False) as (src,dst):dst.objects=names
        for obj in dst.objects:
            collection.objects.link(obj);parent=obj.parent
            while parent:
                if not parent.users_collection:collection.objects.link(parent)
                parent=parent.parent
        bpy.context.view_layer.update();proof=[]
        for obj in dst.objects:
            before=signature(obj);world=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=world;obj.hide_render=False
            if before!=signature(obj):raise ValueError('World geometry changed while importing')
            proof.append(dict(source_node=obj['source_node'],signature=before))
        records.append(dict(worker=str(worker),model_sha256=digest,objects=proof))
    inventory(directory/'assembled-inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=OUT/'animation-references/composite-frame-0.png',patch_manifest=OUT/'source-states/layers.json')
    validate_catalog(directory/'assembled-inventory/inventory.json',reviewed_catalog())
    bpy.ops.wm.save_as_mainfile(filepath=str(target))
    if sha(base)!=baseline_hash:raise ValueError('Input source changed')
    for r in records:
        if sha(Path(r['worker'])/'model.blend')!=r['model_sha256']:raise ValueError('Worker mutated')
    write_json(directory/'assembled-input.json',dict(status='Private93-group full input; registered workers appended unchanged, no user approval implied',model_sha256=sha(target),catalog_sha256=sha(reviewed_catalog()),base=str(base),base_sha256=baseline_hash,records=records))
    print(target)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
