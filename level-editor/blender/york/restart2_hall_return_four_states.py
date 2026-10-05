"""Propagate the reviewed hall end-wall correction to exact archived cover states."""
import argparse,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement';WORK=BASE/'restart2'
p=argparse.ArgumentParser();p.add_argument('state',choices=['initial-initial','initial-applied','applied-initial','applied-applied']);args=p.parse_args(sys.argv[sys.argv.index('--')+1:]);OUT=WORK/'hall-return-four-states-v1'/args.state
if OUT.exists():raise FileExistsError(OUT)
if shutil.disk_usage(ROOT).free<25*1024**3:raise RuntimeError('Disk below25GiB')
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
control=WORK/'hall-return-control-v6-arch-source/model.blend';assert sha(control)=='e498a69fe94a36ba9e039f826a9b4c8ed5706ed3026a3a9e774321b74562dc58'
source=WORK/'hall-four-state-review-v1/approval-batch-v3'/f'{args.state}.blend'
approved={'initial-initial':'92c9a34231e1e691dfbf932cf50b66fb0ef490934cd3403f7e196eb267bd3329','initial-applied':'fb7c4e9cd64e2e60750dcef1b77fa5984a609a1e5f49f179dd02361a597b2632','applied-initial':'32907608194e940e3e46bff8034dffc48623028b920ee44ac693fcb4d85d7b8f','applied-applied':'3b35a7249a973a600d6f60cd7f1d1999c4f4eba36f2565fdbe851865e29ff75a'}
assert sha(source)==approved[args.state]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE/'tooling/current.json').read_text())['directory'])
import bpy
from mathutils import Matrix
from refinement_workspace import _geometry
from source_projection_bake import bake
from render_multiview_asset import render
bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();working=bpy.data.collections['york Working']
target=next(o for o in working.all_objects if o.type=='MESH' and o.get('source_node')=='building-791');geometry={o.name:_geometry(o) for o in scene.objects if o!=target};allowed={'building-791','building-770','building-805','building-806'}
if args.state.endswith('-applied'):allowed.add('scenery-york-great-hall-northwest-arch')
def appearance(o):
 if o.type!='MESH':return None
 return {'materials':[m.name if m else None for m in o.data.materials],'uv':[[list(d.uv) for d in layer.data] for layer in o.data.uv_layers],'hidden':o.hide_render}
outside={o.name:appearance(o) for o in scene.objects if o.get('source_node') not in allowed}
with bpy.data.libraries.load(str(control),link=False) as (available,loaded):loaded.objects=[target.name]
imported=loaded.objects[0]
if imported is None:raise ValueError('Missing reviewed791')
target.data=imported.data.copy();target.parent=None;target.matrix_world=Matrix.Identity(4);bpy.data.objects.remove(imported,do_unlink=True);bpy.context.view_layer.update();OUT.mkdir(parents=True)
a,c=args.state.split('-');art=WORK/f'hall-cover-source-combinations-v1/patch001-{a}_patch002-{c}.png';hall_nodes=['building-791']+(['scenery-york-great-hall-northwest-arch'] if c=='applied' else [])
bake('york',art,OUT/'hall-source-projection.json',receiver_nodes=hall_nodes,receiver_asset_id='york-castle-great-hall',projection_label='hall-semantic-'+args.state,source_mask_manifest=WORK/'hall-source-authority-v2/source-masks.json',texels_per_unit=2,preserve_authored=False)
for nodes in [['building-770'],['building-805','building-806']]:
 owners={o.get('asset_group') for o in working.all_objects if o.get('source_node') in nodes}
 assert len(owners)==1
 bake('york',art,OUT/(nodes[0]+'-context-source.json'),receiver_nodes=nodes,receiver_asset_id=next(iter(owners)),projection_label='private-newly-exposed-context-'+args.state,texels_per_unit=2,preserve_authored=False)
assert geometry=={o.name:_geometry(o) for o in scene.objects if o!=target}
assert outside=={o.name:appearance(o) for o in scene.objects if o.get('source_node') not in allowed}
bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'geometry-guards.json').write_text(json.dumps({'status':'Private four-state correction, pending grouped geometry approval','state':args.state,'approved_base_sha256':sha(source),'reviewed_control_sha256':sha(control),'model_sha256':sha(OUT/'model.blend'),'changed_geometry_source_nodes':['building-791'],'outside_geometry_unchanged':len(geometry),'outside_uv_material_slots_visibility_unchanged':len(outside),'source_refreshed_nodes':sorted(allowed),'source_authority_sha256':sha(WORK/'hall-source-authority-v2/source-masks.json'),'context_scope':'Native770/805/806 source-only refresh for newly exposed rays, same state artwork; unrefined proxies not part of hall approval','api_used':False,'native_791_geometry':_geometry(target)},indent=2)+'\n')
render(WORK/'hall-textures-v1'/args.state/'experiment/views.json',OUT/'actual',width=384)
