"""Build one private hall geometry state for the two independent native covers."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version',default='hall-states-v1')
parser.add_argument('--patch001',choices=('initial','applied'),required=True)
parser.add_argument('--patch002',choices=('initial','applied'),required=True)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
destination=OUT/'restart2'/args.version/(args.patch001+'-'+args.patch002)
if destination.exists():raise FileExistsError(destination)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
import bmesh
import numpy as np
from mathutils import Matrix
from source_projection_bake import bake

source=OUT/'restart2/hall-arch-v1/model.blend'
source_image=OUT/'restart2/hall-cover-source-combinations-v1'/f'patch001-{args.patch001}_patch002-{args.patch002}.png'
bpy.ops.wm.open_mainfile(filepath=str(source))
scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene
bpy.context.view_layer.update()
working=bpy.data.collections['york Working']
objects=[o for o in working.all_objects if o.type=='MESH']
groups={'york-castle-great-hall','york-castle-main-keep','york-castle-east-round-tower'}
def fingerprint(obj):
    return {'points':[list(obj.matrix_world@v.co) for v in obj.data.vertices],
            'faces':[list(f.vertices) for f in obj.data.polygons],
            'uv':[[list(d.uv) for d in l.data] for l in obj.data.uv_layers],
            'materials':[m.name if m else None for m in obj.data.materials],
            'hidden':obj.hide_render}
outside={o.name:fingerprint(o) for o in objects if o.get('asset_group') not in groups}
roof=next(o for o in objects if o.get('source_node')=='building-799')
for node,state in [('building-830',args.patch001),('building-831',args.patch001),('building-832',args.patch002)]:
    found=[o for o in objects if o.get('source_node')==node]
    if len(found)!=1:raise ValueError(f'Ambiguous native cover {node}')
    found[0].hide_render=state=='applied'
level_path=OUT/'baseline/york.rhp.json'
level=json.loads(level_path.read_text())
quad=np.array([[p['x'],p['y'],p['z_top']] for p in level['sight_obstacles'][799]['points']])
sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
def point(u,v):return (1-u)*((1-v)*quad[3]+v*quad[2])+u*((1-v)*quad[0]+v*quad[1])
pieces=[('retained-rear-strip',0,1,0,.27)]
if args.patch001=='initial' or args.patch002=='initial':pieces.append(('northwest-cover-cap',0,.15,.27,1))
if args.patch002=='initial':pieces.append(('main-room-cover',.15,1,.27,1))
vertices=[];faces=[];profiles=[]
def append_volume(profile,bottom):
    start=len(vertices);n=len(profile)
    vertices.extend((x,-y/sine,z/cosine) for (x,y,_),z in zip(profile,bottom))
    vertices.extend((x,-y/sine,z/cosine) for x,y,z in profile)
    local=[tuple(reversed(range(n))),tuple(n+i for i in range(n))]
    local += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    faces.extend(tuple(start+i for i in f) for f in local)
def mesh_for(name):
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges):raise ValueError(f'Open state component {name}')
    bm.to_mesh(mesh);bm.free();mesh.uv_layers.new(name='UVMap')
    return mesh
for name,u0,u1,v0,v1 in pieces:
    profile=[tuple(point(u,v)) for u,v in ((u0,v0),(u1,v0),(u1,v1),(u0,v1))]
    append_volume(profile,[z-2.5 for x,y,z in profile])
    profiles.append({'component':name,'profile_game':profile})
roof.data=mesh_for('Hall roof native cover sections');roof.parent=None;roof.matrix_world=Matrix.Identity(4)
upper_wall=None
if args.patch002=='initial':
    # Reconcile the revealed low cut wall with the covered roof. It must not
    # leave an air gap where the covered artwork has an upper timber storey.
    wall=json.loads((OUT/'restart2/hall-room-v1/geometry.json').read_text())
    low=next(r['profile_game'] for r in wall['changes'] if r['node']=='building-793')
    plane=np.linalg.solve(np.column_stack((quad[:3,0],quad[:3,1],np.ones(3))),quad[:3,2])
    high=[(x,y,float(plane@[x,y,1])-2.5) for x,y,z in low]
    if any(p[2]<=q[2] for p,q in zip(high,low)):raise ValueError('Covered wall cap below cut wall')
    vertices=[];faces=[];append_volume(high,[z for x,y,z in low])
    upper_wall=bpy.data.objects.new('Castle great hall / Covered upper front wall',mesh_for('Covered upper front wall'))
    working.objects.link(upper_wall)
    upper_wall['source_node']='scenery-york-great-hall-upper-front-wall'
    upper_wall['asset_group']='york-castle-great-hall'
    upper_wall['refinement_note']='Visible only while native cover002 is initial.'
bpy.context.view_layer.update();destination.mkdir(parents=True)
for group in sorted(groups):
    nodes=sorted({o['source_node'] for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==group and not o.hide_render})
    bake('york',source_image,destination/(group+'-projection.json'),receiver_nodes=nodes,
         receiver_asset_id=group,projection_label='private-native-cover-geometry',texels_per_unit=2,preserve_authored=False)
if outside!={o.name:fingerprint(o) for o in objects if o.get('asset_group') not in groups}:
    raise ValueError('Cover state altered unrelated context')
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'),compress=True)
config=json.loads((OUT/'restart2/hall-arch-v1/workspace.json').read_text())
config['source_path']=str(source_image)
if upper_wall:config['part_ids'].append(upper_wall['source_node'])
(destination/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(destination/'geometry.json').write_text(json.dumps({
    'status':'HOLD: native-state geometry control awaiting full visual review',
    'states':{'patch001':args.patch001,'patch002':args.patch002},
    'source_model_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
    'source_image_sha256':hashlib.sha256(source_image.read_bytes()).hexdigest(),
    'roof_pieces':profiles,'covered_upper_front_wall':bool(upper_wall),
    'roof_conditions':{'retained-rear-strip':'always','northwest-cover-cap':'patch001 initial OR patch002 initial','main-room-cover':'patch002 initial'},
    'outside_preserved':len(outside),
    'limitations':['Northwest cap and upper wall remain inferred geometry controls.','Keep and east tower are context proxies, not approved refined state geometry.','Native candle animations remain a separate integration step.']},indent=2)+'\n')
