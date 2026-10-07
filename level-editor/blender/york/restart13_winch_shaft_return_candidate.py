"""Save a private shaft-aligned return for visual diagnosis of its frame entry."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-components-source-v2';OUT=WORK/'winch-shaft-return-candidate-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
recipe=Path(__file__).with_name('restart13_winch_return_study.py');defs=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)];exec(compile(ast.Module(body=defs,type_ignores=[]),str(recipe),'exec'));center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0))
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();old=[o for o in scene.objects if o.name.startswith('Suspended chain link')];iron=old[0].data.materials[0];guard={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in old}
for o in old:bpy.data.objects.remove(o,do_unlink=True)
pose,params=route(9.4,2399.5);local=[];surface=[];rx=1.2;perimeter=2*math.pi*rx+4*(3-rx)
for j in range(32):
 p,n=capsule(j*perimeter/32,rx)
 for k in range(8):phi=k*math.tau/8;local.append(p+n*(.4*math.cos(phi))+Vector((0,0,.4*math.sin(phi))))
for j in range(32):
 for k in range(8):surface.append((j*8+k,((j+1)%32)*8+k,((j+1)%32)*8+(k+1)%8,j*8+(k+1)%8))
def own(o):
 o['source_node']='scenery-york-castle-winch';o['asset_group']='york-castle-winch';o['native_patch']='patch-004';o.data.materials.append(iron)
for i in range(72):
 p,r=pose(i*spacing+5.5/c,i);mesh=bpy.data.meshes.new(f'Shaft return closed link {i:03d}');mesh.from_pydata(local,[],surface);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();o=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(o);o.location=p;o.rotation_euler=r.to_euler();own(o)
a=Vector(params['left_station'])+up*params['straight_height'];b=Vector(params['right_station'])+up*params['straight_height']
for name,radius,length in [('Inferred upper return barrel',7,(b-a).length),('Inferred upper return shaft',.9,(b-a).length+6)]:
 bpy.ops.mesh.primitive_cylinder_add(vertices=24,radius=radius,depth=length,location=(a+b)/2);o=bpy.context.object;o.name=name;o.rotation_euler=(b-a).to_track_quat('Z','Y').to_euler();own(o)
bpy.context.view_layer.update();assert guard=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in guard};OUT.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'proposal.json').write_text(json.dumps({'status':'Private visual diagnostic HOLD; no geometry or mechanism approval','source_model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'outside_geometry_appearance_exact':len(guard),'path':params,'profile':{'shape':'capsule','rx':rx,'ry':3,'wire_radius':.4},'limitations':['Front left support and top saddle intersect the chain entry; not mechanically final.','Hidden upper barrel/bearings are inference and need room attachment.','Native strand positions converge during motion; this is a final static hypothesis, not all45frame matching.','No chain source material projection, fill, state integration or publication.']},indent=2)+'\n');print('SHAFT RETURN PRIVATE DIAGNOSTIC SAVED')
