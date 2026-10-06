"""Build a private inferred portcullis and inspect both endpoints with gatehouse context."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
DEST=WORK/'restart2/gate-geometry-v10'
if DEST.exists(): raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector, Matrix
from PIL import Image, ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_camera_audit import audit_manifest, labeled_copy

sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
source=WORK/'grounding/york-grounded.blend'
bpy.ops.wm.open_mainfile(filepath=str(source))
bpy.context.view_layer.update()
nodes={f'building-{i:03d}' for i in (763,764,765,774,776,777,778,779)}
context=[]
for o in bpy.data.collections['york Working'].all_objects:
    if o.type=='MESH' and o.get('source_node',o.name) in nodes and not o.hide_render:
        mesh=o.data.copy(); matrix=o.matrix_world.copy()
        context.append((o.name,o.get('source_node',o.name),mesh,matrix))
assert {r[1] for r in context}==nodes, [(r[0],r[1]) for r in context]
assert len(context)==8, 'Reference copies must not enter the working context'
# Only the eight gatehouse receivers survive in the private scene. Their evaluated
# world coordinates are frozen before detaching any parent/context hierarchy.
for o in list(bpy.data.objects): bpy.data.objects.remove(o,do_unlink=True)
scene=bpy.context.scene;scene.name='York gate hypothesis'
def material(name,color):
    m=bpy.data.materials.new(name);m.diffuse_color=(*color,1);m.use_nodes=True
    bs=m.node_tree.nodes.get('Principled BSDF');bs.inputs['Base Color'].default_value=(*color,1);bs.inputs['Roughness'].default_value=.8
    return m
gray=material('Unrefined gatehouse context',(.31,.35,.39))
wood=material('Unknown portcullis appearance - diagnostic ochre',(.42,.27,.13))
context_objects=[];context_evidence=[]
for name,node,mesh,matrix in context:
    o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o);o.matrix_world=matrix
    o['source_node']=node;o['asset_group']='york-castle-west-gatehouse'
    o.data.materials.clear();o.data.materials.append(gray)
    context_objects.append(o)
    context_evidence.append({'node':node,'matrix_world':[list(r) for r in matrix],
      'world_vertices_sha256':hashlib.sha256(json.dumps([list(matrix@v.co) for v in mesh.vertices]).encode()).hexdigest()})
bpy.context.view_layer.update()
assert all(max(abs(o.matrix_world[r][c]-row[3][r][c]) for r in range(4) for c in range(4))<1e-6 for o,row in zip(context_objects,context))
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
def world(x,y,z):return Vector((x,-y/s,z/c))
# Door midpoints bracket the visible gate plane. Width beyond the source-visible
# sliver is an explicit hypothesis; no native obstacle or collision is invented.
a=world(2336.8003,1012.82166,90.00101);b=world(2377.6338,1053.655,90.00101)
axis=(b-a).normalized();normal=axis.cross(Vector((0,0,1))).normalized()
vertices=[];faces=[]
def beam(start,end,width,depth):
    direction=(end-start).normalized()
    side=normal*depth/2
    cross=direction.cross(normal).normalized()*width/2
    n=len(vertices)
    for p in (start,end):
        vertices.extend([p-side-cross,p+side-cross,p+side+cross,p-side+cross])
    faces.extend([tuple(n+i for i in f) for f in ((0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7))])
height=62/c
upright_x=[2336.8003]+[2339.9+i*4.5 for i in range(9)]+[2377.6338]
for x in upright_x:
    p=a+(b-a)*(x-2336.8003)/(2377.6338-2336.8003)
    beam(p,p+Vector((0,0,height)),2.3,2.1)
for i in range(6):
    z=Vector((0,0,(8.5+i*9.5)/c));beam(a+z,b+z,3.5,2.0)
mesh=bpy.data.meshes.new('Inferred solid gate lattice');mesh.from_pydata(vertices,[],faces);mesh.update()
gate=bpy.data.objects.new('scenery-york-castle-portcullis',mesh);scene.collection.objects.link(gate)
gate['source_node']=gate.name;gate['asset_group']='york-castle-portcullis';gate['asset_name']='Castle courtyard portcullis';gate['part_name']='Lifting grille'
gate['native_patch']='patch-000';gate['geometry_status']='unapproved hypothesis';gate.data.materials.append(wood)
# A recessed stone return belongs to the gatehouse, never to the movable grille.
# Its visible left jamb follows the narrow stone boundary beside the gate sprite;
# the occluded crown/right return are explicitly inferred to meet the old lintel.
arch_profile=[(2336.8003,90.00101),(2336.8003,192.08401),(2377.6338,192.08401),(2377.6338,90.00101),
              (2371.0,90.00101),(2371.0,151),(2368,161),(2362,168),(2355,170),
              (2349,166),(2345,157),(2343,150),(2343,90.00101)]
arch_points=[world(x,1012.82166+(x-2336.8003),z) for x,z in arch_profile]
assert normal.dot(Vector((0,-c,s)))>0, 'Return extrusion must face the native camera'
arch_vertices=arch_points+[p+normal*5 for p in arch_points]
n=len(arch_points);arch_faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]
arch_faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
arch_mesh=bpy.data.meshes.new('Scoped recessed arch return hypothesis');arch_mesh.from_pydata(arch_vertices,[],arch_faces);arch_mesh.update()
arch=bpy.data.objects.new('building-778-portcullis-jamb-return',arch_mesh);scene.collection.objects.link(arch);arch.data.materials.append(gray)
arch['source_node']='building-778';arch['projection_component']='portcullis-jamb-return';arch['asset_group']='york-castle-west-gatehouse';arch['geometry_status']='unapproved source-led correction hypothesis'
context_objects.append(arch)
level=json.loads((WORK/'baseline/york.rhp.json').read_text())
floor_evidence=[]
def clip(poly,axis,value,above):
    result=[]
    for first,second in zip(poly,poly[1:]+poly[:1]):
        inside=lambda p:p[axis]>=value if above else p[axis]<=value
        if inside(first):result.append(first)
        if inside(first)!=inside(second):
            t=(value-first[axis])/(second[axis]-first[axis]);result.append(tuple(first[i]+t*(second[i]-first[i]) for i in range(2)))
    return result
for index in (92,98):
    obstacle=level['sight_obstacles'][index];poly=[(p['x'],p['y']) for p in obstacle['points']]
    assert all(abs(p['z_top']-90.00101)<1e-6 for p in obstacle['points'])
    for axis,value,above in ((0,2280,True),(0,2440,False),(1,985,True),(1,1110,False)):poly=clip(poly,axis,value,above)
    mesh=bpy.data.meshes.new(f'Floor contact proxy {index}');mesh.from_pydata([world(x,y,90.00101) for x,y in poly],[],[list(range(len(poly)))]);mesh.update()
    o=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(o);o.data.materials.append(gray);context_objects.append(o)
    o['source_node']=f'building-{index:03d}';o['diagnostic_only']=True
    floor_evidence.append({'obstacle':index,'projection_area':obstacle['projection_area'],'top':90.00101,'clip_game_xy':[2280,985,2440,1110],'scope':'Cropped exact flat native obstacle top; context only, not reusable asset geometry'})
scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=8;scene.cycles.use_denoising=False
scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.film_transparent=True
scene.render.resolution_x=320;scene.render.resolution_y=384
scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
scene.world=bpy.data.worlds.new('Gate review world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.6,.6,.6,1);scene.world.node_tree.nodes['Background'].inputs[1].default_value=.7
bpy.data.orphans_purge(do_recursive=True)
ld=bpy.data.lights.new('Neutral review sun','SUN');lo=bpy.data.objects.new(ld.name,ld);scene.collection.objects.link(lo);lo.rotation_euler=(.5,-.6,-.4);ld.energy=2
DEST.mkdir(parents=True)
def cameras(center,scale,prefix):
    rows=[];names={}
    for i in range(8):
        yaw=math.radians(i*45);back=Vector((math.sin(yaw)*c,-math.cos(yaw)*c,s))
        data=bpy.data.cameras.new(f'{prefix}-{i}');o=bpy.data.objects.new(data.name,data);scene.collection.objects.link(o)
        data.type='ORTHO';data.ortho_scale=scale;data.clip_end=20000;o.location=center+back*10000;o.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler()
        bpy.context.view_layer.update();names[f'view-{i}']=o.name
        rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in o.matrix_world],'ortho_scale':scale})
    return names,rows
gate_center=(a+b)/2+Vector((0,0,(height+57/c)/2))
isolated,iso_rows=cameras(gate_center,200,'Gate')
points=[o.matrix_world@v.co for o in context_objects for v in o.data.vertices]
low=Vector(tuple(min(p[i] for p in points) for i in range(3)));high=Vector(tuple(max(p[i] for p in points) for i in range(3)))
joint,joint_rows=cameras((low+high)/2,max((high-low).length,350)*1.15,'Contact')
native_data=bpy.data.cameras.new('Native crop');native=bpy.data.objects.new(native_data.name,native_data);scene.collection.objects.link(native)
native_data.type='ORTHO';native_data.ortho_scale=250;native_data.clip_end=20000
native.location=world(2360,905,0)+Vector((0,-c,s))*10000;native.rotation_euler=Vector((0,c,-s)).to_track_quat('-Z','Y').to_euler()
def review(folder,names,rows):
    folder.mkdir();manifest={'layout':{'columns':4,'rows':2},'views':rows};(folder/'views.json').write_text(json.dumps(manifest,indent=2)+'\n');audit_manifest(folder/'views.json')
    render_views(scene.name,names,folder/'renders',modes=('textured',),width=320)
    tiles=[Image.open(folder/f'renders/view-{i}-textured.png').convert('RGBA') for i in range(8)]
    sheet=Image.new('RGBA',(1280,768))
    for i,t in enumerate(tiles):sheet.paste(t,((i%4)*320,(i//4)*384))
    sheet.save(folder/'solid8.png');labeled_copy(folder/'solid8.png',folder/'solid8-native-labeled.png')
states=[]
for state,lift,source_frame in [('covered',0,'initial'),('raised',57,'transition-44')]:
    out=DEST/state;out.mkdir();gate.location.z=lift/c;bpy.context.view_layer.update()
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
    for o in context_objects:o.hide_render=True
    review(out/'isolated',isolated,iso_rows)
    arch.hide_render=False
    review(out/'gate-and-return',isolated,iso_rows)
    for o in context_objects:o.hide_render=False
    review(out/'contact',joint,joint_rows)
    scene.render.resolution_x=440;scene.render.resolution_y=500
    render_views(scene.name,{'native':native.name},out/'native',modes=('textured',),width=440)
    orig=Image.open(WORK/f'restart2/gate-source-study-v2/{source_frame}.png').convert('RGB').resize((440,500),Image.Resampling.NEAREST)
    actual=Image.open(out/'native/native-textured.png').convert('RGBA');sheet=Image.new('RGB',(880,524),'#182028');sheet.paste(orig,(0,24));sheet.paste(actual,(440,24),actual)
    d=ImageDraw.Draw(sheet);d.text((5,6),'Original native source (mechanism separate)',fill='white');d.text((445,6),'Gate hypothesis + unrefined gray context',fill='white');sheet.save(out/'native-comparison.png')
    scene.render.resolution_x=320;scene.render.resolution_y=384
    states.append({'state':state,'lift_game_pixels':lift,'model_sha256':sha(out/'model.blend')})
(DEST/'geometry-proposal.json').write_text(json.dumps({'status':'HOLD pending self-review','source_blend':str(source),'source_sha256':sha(source),'source_study_sha256':sha(WORK/'restart2/gate-source-study-v2/manifest.json'),'context':context_evidence,'floor_contact':floor_evidence,'states':states,'scoped_gatehouse_correction':{'owner':'york-castle-west-gatehouse','source_node':'building-778','component':'portcullis-jamb-return','profile_game_xz':arch_profile,'extrusion_world_depth':5,'source_basis':'Visible narrow stone jamb immediately left of patch000 and curved head; hidden crown/right return inferred to join lintel777 at192.08401','approval':'NONE; requires combined geometry review separately from unchanged context'},'inferences':['Hidden gate extends across exact floor92/98 seam; Three visible uprights and six crossbars traced from the native sliver; regular hidden upright continuation and edge posts inferred.','Full solid lattice translates upward57 game pixels; gatehouse supplies occlusion, gate is never cut to sprite bounds.','No source texture projected; ochre identifies unknown gate geometry.','Existing gatehouse geometry is unrefined context; newly authored recessed arch return is explicitly included in correction scope. Mechanism patch004 excluded.'],'gameplay':'Native doors13/14 association retained in source study only; no descriptor, catalog or live changes.'},indent=2)+'\n')
print('GATE GEOMETRY PROPOSAL COMPLETE',flush=True)
