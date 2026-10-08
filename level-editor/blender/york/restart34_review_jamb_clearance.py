"""Save and review a private, camera-ray-preserving jamb clearance candidate."""
import hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
W=ROOT/'level-editor/work/york-refinement/restart2'
OUT=W/'jamb-clearance-candidate-v1'
assert OUT.exists() and not (OUT/'validation.json').exists()
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageChops
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
from workspace_components import appearance_state
from render_multiview_asset import render
from render_views import render_views
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
source=W/'jamb-textures-v1/model.blend'
assert sha(source)=='b26c36588a7a8bdcd7fdd8a5bdc5d576131cc465ab2b7c68ec0d4d7613587ec7'
plan_path=W/'jamb-hidden-clearance-plan-v2/proposal.json'
plan=json.loads(plan_path.read_text())
audit=json.loads((W/'gate-saved-contact-audit-v1/report.json').read_text())
motion=json.loads((W/'gate-motion-proposal-v2/motion.json').read_text())
manifest=W/'jamb-texture-inputs-v3/experiment/views.json'
assert manifest.exists()
bpy.ops.wm.open_mainfile(filepath=str(source))
scene=bpy.context.scene
scene.render.threads_mode='FIXED';scene.render.threads=2
bpy.context.view_layer.update()
jamb=bpy.data.objects['building-778-portcullis-jamb-return']
world=lambda o:[list(o.matrix_world@v.co) for v in o.data.vertices]
assert world(jamb)==plan['old_vertices_world'], 'Final textured jamb differs from audited geometry'
front=[list(v.co) for v in jamb.data.vertices][13:]
faces=[list(p.vertices) for p in jamb.data.polygons]
uvs=[[(list(l.uv)) for l in layer.data] for layer in jamb.data.uv_layers]
appearance=appearance_state(jamb)
outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o!=jamb}
bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'))
scene=bpy.context.scene;jamb=bpy.data.objects['building-778-portcullis-jamb-return']
scene.render.threads_mode='FIXED';scene.render.threads=2
assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o!=jamb}
assert appearance_state(jamb)==appearance
assert [list(v.co) for v in jamb.data.vertices][13:]==front
scene.render.resolution_x=220;scene.render.resolution_y=250
render_views(scene.name,{'native':'Native crop'},OUT/'native-after',modes=('textured',),width=440)
before=Image.open(OUT/'native-before/native-textured.png').convert('RGBA')
after=Image.open(OUT/'native-after/native-textured.png').convert('RGBA')
assert before.size==after.size
import numpy as np
difference=np.abs(np.asarray(before).astype(int)-np.asarray(after).astype(int))
source_mask=Image.open(W/'jamb-source-probe-v1/visible-jamb-domain.png').convert('L').resize(before.size,Image.Resampling.NEAREST)
known=np.asarray(source_mask)>0
pixel_report={'all_changed_pixels':int(np.any(difference>0,axis=2).sum()),'max_channel_difference':int(difference.max()),'known_source_changed_pixels':int(np.any(difference>0,axis=2)[known].sum()),'known_source_max_channel_difference':int(difference[known].max())}
# Raster edge roundoff is disclosed separately from exact texture/UV preservation.
render(manifest,OUT/'actual',modes=('solid','textured'),width=320)
for mode in ('solid','textured'):
    views=[Image.open(OUT/f'actual/view-{i}-{mode}.png').convert('RGBA') for i in range(8)]
    sheet=Image.new('RGBA',(views[0].width*4,views[0].height*2))
    for i,view in enumerate(views):sheet.paste(view,((i%4)*view.width,(i//4)*view.height))
    sheet.save(OUT/f'actual/{mode}-eight.png')
jv=[Vector(v) for v in world(jamb)];jf=[list(p.vertices) for p in jamb.data.polygons]
jt=BVHTree.FromPolygons(jv,jf)
gv=[Vector(v) for v in audit['geometry_world']['gate_vertices']];gf=audit['geometry_world']['gate_faces']
normal=Vector(plan['normal_world']);base=Vector(plan['old_vertices_world'][0])
rows=[]
for row in motion['rows']:
    vs=[v+Vector((0,0,row['nominal_lift_world_z'])) for v in gv]
    pairs=BVHTree.FromPolygons(vs,gf).overlap(jt)
    gap=min((v-base).dot(normal) for v in jv)-max((v-base).dot(normal) for v in vs)
    assert not pairs and gap>.049
    rows.append({'frame':row['frame'],'triangle_intersection_pairs':len(pairs),'normal_separation':gap,'lift_world_z':row['nominal_lift_world_z']})
report={'status':'SAVED_PRIVATE_CANDIDATE_REQUIRES_VISUAL_REVIEW_AND_NEW_GEOMETRY_APPROVAL','source_model_sha256':sha(source),'saved_model_sha256':sha(OUT/'model.blend'),'proposal_sha256':sha(plan_path),'unchanged_front_vertices':13,'changed_back_vertices':13,'outside_objects_exact':len(outside),'uv_loops_and_appearance_exact':True,'native_raster_comparison':pixel_report,'contacts':rows,'minimum_all_pose_gap':min(r['normal_separation'] for r in rows),'approval_scope':'Changed inferred jamb back profile and separately inferred 45-pose gate motion; no previous geometry approval inherited.'}
(OUT/'validation.json').write_text(json.dumps(report,indent=2)+'\n')
assert sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file())<32*1024**2
print(json.dumps({'out':str(OUT),'native':pixel_report,'gap':report['minimum_all_pose_gap']}))
