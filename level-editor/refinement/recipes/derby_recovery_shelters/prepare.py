"""Recover approved shelter geometry with current native ownership and sunlight."""
import bpy,sys,json,hashlib
from pathlib import Path
ROOT=Path.cwd();sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_review import render_review
from review_sunlight import DEFAULT_LIGHTING
from refinement_workspace import _geometry
BASE=ROOT/'level-editor/work/needs-review/derby-refinement/round-2'
name=sys.argv[sys.argv.index('--')+1]
old=BASE/name; manifest=json.loads((old/'views.json').read_text())
dest=BASE/'recovery-shelters-20260923'/manifest['asset_id'];dest.mkdir(parents=True,exist_ok=True)
assignments={
 'derby-east-courtyard-south-shelter': [('building-067',[13],[80]),('building-068',[12],[80])],
 'derby-east-wall-landing':[('building-069',[14],[80,82,83])],
 'derby-lower-northwest-cottage-barrel':[('building-062',[28],[])],
}[manifest['asset_id']]
mask={'version':1,'mask_inventory':str(ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.d/masks/manifest.json'),'projections':{'exterior':{'source_sha256':manifest['source_sha256'],'state':'Covered initial mission artwork; native base-layer silhouette','assignments':[]}}}
for node,indices,exclusions in assignments:
 a={'source_node':node,'reviewed':True,'mask_indices':indices,'layer':0}
 if exclusions:a.update(exclude_mask_indices=exclusions,exclusions_reviewed=True,exclusion_reason='Foreground castle curtain/tower/stairs mask excludes stone artwork from the shelter; mesh first-hit visibility remains required.')
 mask['projections']['exterior']['assignments'].append(a)
(dest/'source-masks.json').write_text(json.dumps(mask,indent=2))
scene=bpy.data.scenes[manifest['scene_name']];bpy.context.window.scene=scene;bpy.context.view_layer.update()
before={o.name:_geometry(o) for o in scene.objects}
targets=[o for o in bpy.data.collections[manifest['collection_name']].all_objects if o.type=='MESH' and not o.hide_render and o.get('asset_group')==manifest['asset_id']]
def geo(o):return {'vertices':[list(o.matrix_world@v.co) for v in o.data.vertices],'faces':[list(p.vertices) for p in o.data.polygons]}
approved={o.name:geo(o) for o in targets}
(dest/'approved-geometry.json').write_text(json.dumps(approved))
bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
layers=[l for l in manifest['projection_layers'] if set(l['receiver_nodes']) & {o.get('source_node') for o in targets}]
for layer in layers:
 if layer['source_sha256']==manifest['source_sha256']:layer['projection_label']='exterior'
render_review(dest/'modified-masked-v2',scene_name=manifest['scene_name'],collection_name=manifest['collection_name'],asset_id=manifest['asset_id'],source_path=manifest['source_image'],frame_manifest=manifest,width=manifest['tile_size'][0],height=manifest['tile_size'][1],projection_layers=layers,lighting=DEFAULT_LIGHTING,source_mask_manifest=dest/'source-masks.json',allow_mask_revision=True,allow_projection_revision=True)
assert before=={o.name:_geometry(o) for o in scene.objects}
(dest/'preparation-validation.json').write_text(json.dumps({'geometry_unchanged':True,'objects':len(before),'approved_model':manifest['source_blend'],'approved_model_sha256':hashlib.sha256(Path(manifest['source_blend']).read_bytes()).hexdigest(),'geometry_approval':json.loads((old/'approval.json').read_text()),'old_generation_retained':str(old),'reason_for_regeneration':'Native mask authority and user-selected 48-degree sun'},indent=2))
