"""Render private source-only endpoint fill inputs with unchanged physical geometry."""
import json,hashlib,sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from refinement_review import render_review
from render_slots import acquire,release
from prepare_private_texture_inputs import prepare

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def signature(objects):
    rows=[]
    for o in objects:
        rows.append(dict(name=o.name,matrix=[list(r)for r in o.matrix_world],vertices=[list(v.co)for v in o.data.vertices],faces=[list(p.vertices)for p in o.data.polygons],material_indices=[p.material_index for p in o.data.polygons],materials=[m.name if m else None for m in o.data.materials],uvs={u.name:[list(d.uv)for d in u.data]for u in o.data.uv_layers}))
    return hashlib.sha256(json.dumps(rows,sort_keys=True).encode()).hexdigest()
def prepare_main():
    args=sys.argv[sys.argv.index('--')+1:];assembly,state=args;asset=f'croisement02-{assembly}-{state}';dest=OUT/'restart2-state/private-empty01-inputs-v1'/asset;dest.mkdir(parents=True,exist_ok=False)
    if assembly in ['log-trap','rock-trap']:
        base=OUT/f'{assembly}-state-candidate-v14';record=json.loads((OUT/f'state-target-evidence/{assembly}/manifest.json').read_text());box=record['bbox'];tick=-1 if state=='covered'else (89 if assembly=='log-trap'else 104);source=OUT/f'state-target-evidence/{assembly}/tick-{tick:03}.png';xy=box[:2]
    elif assembly=='net-piege01':
        assert state=='empty-final-0';base=OUT/'restart2-state/net-empty01-v11';fit=json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text());record=next(r for r in fit['records']if r['family']=='piege01'and r['variant']=='e');source=Path(record['source']);assert sha(source)==record['source_sha256'];xy=record['bbox'][:2]
    else:raise ValueError('Unsupported endpoint')
    model=base/'model.blend';expected='3c80a9ce19306de621d170713663159059493c60a3b5131eb415bcd47a7dedf2';assert sha(model)==expected
    bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();objects=sorted([o for o in scene.objects if o.type=='MESH'and(assembly=='net-piege01'or o.get('state_endpoint')==state)],key=lambda o:o.name);assert objects;before=signature(objects)
    collection=bpy.data.collections.new('Private state endpoint Working');scene.collection.children.link(collection)
    for obj in scene.objects:
        if obj.type=='MESH':obj.hide_render=obj not in objects
    for index,obj in enumerate(objects):
        if obj.name not in collection.objects:collection.objects.link(obj)
        obj['asset_group']=asset;obj['source_node']=f'{asset}-part-{index:03}';obj.hide_render=False
    canvas=Image.new('RGBA',Image.open(OUT/'baseline/covered.png').size);crop=Image.open(source).convert('RGBA');canvas.alpha_composite(crop,tuple(xy));canvas.save(dest/'source.png');alpha=np.array(canvas)[:,:,3]>0;Image.fromarray(alpha.astype(np.uint8)*255).save(dest/'source-alpha.png')
    inventory=dict(masks=[dict(index=0,box_top_left=[0,0],box_size=list(canvas.size),png='source-alpha.png')]);(dest/'mask-inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')
    masks=dict(version=1,mask_inventory='mask-inventory.json',projections=dict(exterior=dict(source_sha256=sha(dest/'source.png'),state=f'{assembly} {state}: exact native endpoint alpha, reviewed source evidence only',assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[0])])));(dest/'source-masks.json').write_text(json.dumps(masks,indent=2)+'\n')
    assert before==signature(objects);bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
    frames=render_review(dest/'modified',scene_name=scene.name,collection_name=collection.name,asset_id=asset,source_path=dest/'source.png',source_mask_manifest=dest/'source-masks.json',width=384,height=384,framing_padding=1.15)
    assert before==signature(objects);assert sha(model)==expected
    proof=dict(status='Private preparation only; no user approval or generation authorization',source_model=str(model),source_model_sha256=expected,prepared_model_sha256=sha(dest/'model.blend'),geometry_uv_material_signature=before,geometry_uv_materials_unchanged=True,changes='Only private state visibility, collection membership and source ownership metadata; original worker untouched.',source_image=str(source),source_sha256=sha(source),source_placement=xy,source_alpha_pixels=int(alpha.sum()),known_projection='Exact native alpha; first-hit physical receiver test and source normal threshold. Unknown backs remain editable.',object_names=[o.name for o in objects],generation_authorized=False)
    (dest/'derivation.json').write_text(json.dumps(proof,indent=2)+'\n');prepare(dest,sha(dest/'model.blend'),dest/'private-inputs');print(asset,'private inputs prepared')
if __name__=='__main__':
    acquire()
    try:prepare_main()
    finally:release()
