"""Private closed hall shell from reviewed ownership and native upper-floor datum."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
DEST=OUT/'restart2/hall-shell-v1'
if DEST.exists():raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import bmesh
from mathutils import Matrix

source=OUT/'restart2/hall-furniture-v5/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source))
DEST.mkdir(parents=True)
native=json.loads((OUT/'baseline/york.rhp.json').read_text())
catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text())
for n in (769,795):
    partition=next(p for p in catalog['partitions'] if p['source_node']==f'building-{n}')
    if partition['axis_coefficients']!=[.305,1.] or partition['boundaries']!=[1838.5]:
        raise ValueError('Reviewed hall/tower boundary changed')
objects=[o for o in bpy.data.collections['york Working'].all_objects if o.type=='MESH' and not o.hide_render]
targets=[o for o in objects if o.get('asset_group')=='york-castle-great-hall' and o.get('source_node') in {f'building-{n}' for n in (769,793,795,799)}]
if len(targets)!=4:raise ValueError('Expected four hall shell components')
def fingerprint(o):
    return hashlib.sha256(json.dumps({'v':[list(o.matrix_world@v.co) for v in o.data.vertices],
        'f':[list(f.vertices) for f in o.data.polygons],
        'uv':[[list(d.uv) for d in layer.data] for layer in o.data.uv_layers]},sort_keys=True).encode()).hexdigest()
outside={o.name:fingerprint(o) for o in objects if o not in targets}
sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
rows=[]
for obj in targets:
    n=int(obj['source_node'].split('-')[1]);before=fingerprint(obj)
    profile=[(p['x'],p['y'],p['z_top']) for p in native['sight_obstacles'][n]['points']]
    if n in (769,795):
        clipped=[]
        for a,b in zip(profile,profile[1:]+profile[:1]):
            da=a[1]+.305*a[0]-1838.5;db=b[1]+.305*b[0]-1838.5
            if da<=0:clipped.append(a)
            if (da<0<db) or (db<0<da):
                t=da/(da-db);clipped.append(tuple(a[i]+t*(b[i]-a[i]) for i in range(3)))
        profile=clipped
    count=len(profile)
    if count<3:raise ValueError('Empty hall profile')
    lower=[z-2.5 if n==799 else 90.00101 if n==769 else 225.001 for x,y,z in profile]
    vertices=[(x,-y/sine,z/cosine) for (x,y,_),z in zip(profile,lower)]
    vertices += [(x,-y/sine,z/cosine) for x,y,z in profile]
    faces=[tuple(reversed(range(count))),tuple(count+i for i in range(count))]
    faces += [(i,(i+1)%count,count+(i+1)%count,count+i) for i in range(count)]
    mesh=bpy.data.meshes.new(obj.name+' closed shell')
    mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges):raise ValueError('Open hall shell')
    bm.to_mesh(mesh);bm.free()
    for material in obj.data.materials:mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4)
    rows.append({'source_node':obj['source_node'],'component':obj.get('projection_component'),
                 'before':before,'after':fingerprint(obj),'profile_game':profile,'bottom_game_z':lower})
if outside!={o.name:fingerprint(o) for o in objects if o not in targets}:
    raise ValueError('Changed outside hall shell geometry or UVs')
bpy.context.window.scene=bpy.data.scenes['york Refinement']
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'),compress=True)
config=json.loads((OUT/'geometry-pass-01/assets/york-castle-great-hall/workspace.json').read_text())
(DEST/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(DEST/'geometry.json').write_text(json.dumps({'status':'HOLD: private closed shell, no source projection or appearance approval',
    'changes':rows,'outside_preserved':len(outside),
    'inferred':['Close hall/tower partition with hidden caps.','Roof underside thickness 2.5 game units.'],
    'remaining':['Part791 upper junction still needs source-led reconstruction.','Partial roof/facade cover for patch002 and overlapping patch001.','Revealed room floor/wall and furniture source projection.','Joint hall/tower and terrain review.']},indent=2)+'\n')
