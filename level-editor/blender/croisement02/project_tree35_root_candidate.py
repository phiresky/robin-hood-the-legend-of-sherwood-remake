"""Apply private foreground ownership to rounded oak roots without a texture API."""
import hashlib,json,sys
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry
from source_projection_bake import bake
from render_multiview_asset import render


def geometry(objects):
    return {o.name:hashlib.sha256(repr(([(tuple(v.co)) for v in o.data.vertices],[tuple(f.vertices) for f in o.data.polygons])).encode()).hexdigest() for o in objects}


def main():
    original=tree_workspace(35);parent=OUT/'tree35-root-research/candidate-v4';domain=OUT/'tree35-root-research/source-domain-v1';directory=OUT/'tree35-root-research/candidate-v5';directory.mkdir(exist_ok=False)
    original_hash=sha(original/'model.blend');prior=json.loads((parent/'evidence.json').read_text())
    if sha(parent/'model.blend')!=prior['model_sha256']:raise ValueError('Geometry candidate changed')
    review=json.loads((domain/'source-review.json').read_text())
    if sha(domain/'source-masks.json')!=review['source_mask_manifest_sha256'] or sha(domain/'mask-inventory.json')!=review['mask_inventory_sha256']:raise ValueError('Unreviewed source domain')
    bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==original.name and o.get('projection_component')!='crown'];outside={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};before_geometry=geometry(wood)
    cfg=json.loads((original/'workspace.json').read_text());bake('Croisement02',cfg['source_path'],directory/'source-ownership.json',receiver_nodes=sorted({o['source_node'] for o in wood}),receiver_object_names=[o.name for o in wood],occluder_nodes=sorted({o['source_node'] for o in wood}),projection_label='exterior',preserve_authored=False,source_mask_manifest=str(domain/'source-masks.json'))
    if outside!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood} or geometry(wood)!=before_geometry:raise ValueError('Source bake changed geometry or another owner')
    bpy.ops.wm.save_as_mainfile(filepath=str(directory/'model.blend'))
    scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=8
    for scope,path in [('roots',OUT/'tree35-root-research/baseline-v2/views.json'),('full',original/'modified/views.json')]:
        packet=json.loads(path.read_text());packet.pop('render_object_names',None);packet['source_blend']=str(directory/'model.blend')
        for view in packet['views']:view['crop']=dict(width=320,height=320)
        write_json(directory/f'{scope}-views.json',packet)
        for obj in objects:
            if obj.type=='MESH' and obj.get('asset_group')==original.name:obj.hide_render=(scope=='roots' and obj not in wood)
        render(directory/f'{scope}-views.json',directory/scope,modes=('textured','solid'),width=320)
        for mode in ['textured','solid']:
            sheet=Image.new('RGB',(1280,640))
            for i in range(8):sheet.paste(Image.open(directory/f'{scope}/view-{i}-{mode}.png'),((i%4)*320,(i//4)*320))
            sheet.save(directory/f'{scope}-{mode}.png')
    if sha(original/'model.blend')!=original_hash:raise ValueError('Approved model changed')
    write_json(directory/'evidence.json',dict(status='private geometry plus source-domain correction; awaiting self-review',model_sha256=sha(directory/'model.blend'),original_model=str(original/'model.blend'),original_model_sha256=original_hash,geometry_parent=str(parent/'model.blend'),geometry_parent_sha256=prior['model_sha256'],source_domain=str(domain/'source-review.json'),source_domain_sha256=sha(domain/'source-review.json'),source_manifest_sha256=sha(domain/'source-masks.json'),root_geometry=prior['root_geometry'],outside_appearance_fingerprints=outside,crown_and_other_owner_appearance_unchanged=True,wood_geometry_unchanged_by_bake=True,limitations=['Native85/128/131 foliage and domain505 uncertainty excluded from wood.','Unknown back/root textures remain neutral for future fill after geometry approval.','Voxel union resamples wood; front native appearance must be compared.','No canonical model, user approval or texture fill changed.']))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
