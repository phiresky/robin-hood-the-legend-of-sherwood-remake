"""Bounded private loose-plank candidate, receiver queries and saved-model views."""
import hashlib, json, math, shutil, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
OUT=WORK/'restart2/loose-planks-candidate-v1'
SOURCE=WORK/'grounding/york-grounded.blend'
PLAN=WORK/'restart2/timber-contact-plan-v1/plan.json'
if OUT.exists(): raise FileExistsError(OUT)
def budget():
    used=sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file()) if OUT.exists() else 0
    assert used<32*1024**2
    assert shutil.disk_usage(ROOT).free>10*1024**3+32*1024**2-used
budget()
assert hashlib.sha256(PLAN.read_bytes()).hexdigest()=='ec34bcd8e3b98fa5a7e5a957e0b55b86684f7f4ae8096ebfe5c720eddad9bf12'
assert hashlib.sha256(SOURCE.read_bytes()).hexdigest()=='32d581cae5d6730aebcf1a1ad177817952afed0c529156edb468dbf3b40045c2'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy, bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image, ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
plan=json.loads(PLAN.read_text()); s=math.sin(math.radians(35)); c=math.cos(math.radians(35))
back=Vector((0,-c,s))
def world(p,z): return [p[0],-(p[1]+c*z)/s,z]
def project(p): return (p[0],-s*p[1]-c*p[2])

# Rectangular rightmost variant, fitted in source pixels, all four marks within2px.
observed=[(1282,959),(1287,964),(1309,948),(1307,945)]
cx=sum(x for x,y in observed)/4;cy=sum(y for x,y in observed)/4
sw=[-1,1,1,-1];sl=[-1,-1,1,1]
u=[sum(q[k]*v for q,v in zip(observed,sw))/4 for k in (0,1)]
v=[sum(q[k]*z for q,z in zip(observed,sl))/4 for k in (0,1)]
best=None
for angle in range(-899,0):
    t=math.radians(angle/10);a=[math.cos(t),-s*math.sin(t)];b=[-math.sin(t),-s*math.cos(t)]
    w=sum(x*y for x,y in zip(u,a))/sum(x*x for x in a)
    length=sum(x*y for x,y in zip(v,b))/sum(x*x for x in b)
    pixels=[[cx+x*w*a[0]+y*length*b[0],cy+x*w*a[1]+y*length*b[1]] for x,y in zip(sw,sl)]
    errors=[math.dist(x,y) for x,y in zip(observed,pixels)]
    if best is None or max(errors)<best['maximum_error']:
        best={'pixels':pixels,'errors':errors,'maximum_error':max(errors),'angle_degrees':angle/10}
assert best['maximum_error']<2
row=next(p for p in plan['pieces'] if p['id']=='pale-4')
row['top_source_polygon']=best['pixels'];row['footprint_world']=[world(p,row['top_z'])[:2] for p in best['pixels']]

bpy.ops.wm.read_factory_settings(use_empty=True)
groups=('West town raised terrain',)
with bpy.data.libraries.load(str(SOURCE),link=False) as (src,dst):
    dst.objects=[n for n in src.objects if n in groups or n.startswith('West town raised terrain / ')]
for o in dst.objects: bpy.context.scene.collection.objects.link(o)
bpy.context.view_layer.update()
ground=[o for o in dst.objects if o.type=='MESH' and not o.hide_render and o.get('source_node')=='building-086']
assert ground
vv=[];ff=[]
for o in ground:
    offset=len(vv);vv.extend(o.matrix_world@v.co for v in o.data.vertices)
    ff.extend(tuple(offset+i for i in p.vertices) for p in o.data.polygons)
ground_tree=BVHTree.FromPolygons(vv,ff)
ground_queries=[]
for p in plan['pieces']:
    if p['bottom_z']!=plan['ground_plane_z']: continue
    corners=p['footprint_world'];center=[sum(v[k] for v in corners)/len(corners) for k in (0,1)]
    samples=corners+[center]+[[(a[k]+b[k])/2 for k in (0,1)] for a,b in zip(corners,corners[1:]+corners[:1])]
    for x,y in samples:
        loc,normal,face,distance=ground_tree.ray_cast(Vector((x,y,1000)),Vector((0,0,-1)))
        assert loc is not None, (p['id'],x,y,'ground miss')
        delta=p['bottom_z']-loc.z
        assert abs(delta)<.002,(p['id'],delta)
        ground_queries.append({'piece':p['id'],'foot':[x,y,p['bottom_z']],
                               'receiver':list(loc),'triangle_or_polygon':face,'delta_z':delta})
# Keep saved candidate small. Receiver geometry is queried above, never altered.
for o in list(bpy.data.objects): bpy.data.objects.remove(o,do_unlink=True)
for collection in (bpy.data.meshes,bpy.data.materials,bpy.data.images):
    for datablock in list(collection):
        if datablock.users==0: collection.remove(datablock)
scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
objects=[]
faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
for p in plan['pieces']:
    verts=[(x,y,z) for z in (p['bottom_z'],p['top_z']) for x,y in p['footprint_world']]
    mesh=bpy.data.meshes.new(p['id']);mesh.from_pydata(verts,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free()
    o=bpy.data.objects.new('Loose planks / '+p['id'],mesh);scene.collection.objects.link(o)
    o['source_node']='building-008';o['asset_group']='york-riverside-loose-planks';o['inferred_individual_board']=True
    objects.append(o)
bpy.context.view_layer.update()
verts=[];polys=[];owners=[]
for o in objects:
    offset=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices)
    for p in o.data.polygons: polys.append(tuple(offset+i for i in p.vertices));owners.append((o.name,p.index))
tree=BVHTree.FromPolygons(verts,polys)
art_path=WORK/'restart2/pair-v14/assets/york-southwest-square-west-house/reference/source.png'
art=Image.open(art_path).convert('RGBA');box=(1240,920,1320,978);width=box[2]-box[0];height=box[3]-box[1]
mask_path=ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/york.rhp.d/masks/000005.png'
mask=Image.open(mask_path).convert('L')
images={key:Image.new('RGBA',(width,height),(128,128,128,255)) for key in owners}
accepted=0;outside_mask=0;mask_uncovered=0
for y in range(box[1],box[3]):
    for x in range(box[0],box[2]):
        mx,my=x-1250,y-929
        masked=0<=mx<mask.width and 0<=my<mask.height and mask.getpixel((mx,my))>0
        origin=Vector((x+.5,-(y+.5)/s,0))+back*1000
        loc,normal,index,distance=tree.ray_cast(origin,-back)
        if loc is None:
            mask_uncovered+=bool(masked);continue
        if not masked: outside_mask+=1;continue
        if normal.dot(back)<.05:continue
        images[owners[index]].putpixel((x-box[0],y-box[1]),art.getpixel((x,y)));accepted+=1
OUT.mkdir();(OUT/'face-source').mkdir()
for o in objects:
    uv=o.data.uv_layers.new(name='Source diagnostic')
    for p in o.data.polygons:
        path=OUT/'face-source'/f'{o.data.name}-{p.index}.png';images[(o.name,p.index)].save(path)
        image=bpy.data.images.load(str(path));image.pack()
        mat=bpy.data.materials.new(f'{o.data.name} face{p.index} source or unknown');mat.use_nodes=True
        nodes=mat.node_tree.nodes;nodes.clear();out=nodes.new('ShaderNodeOutputMaterial');shader=nodes.new('ShaderNodeEmission');tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest'
        mat.node_tree.links.new(tex.outputs['Color'],shader.inputs['Color']);mat.node_tree.links.new(shader.outputs[0],out.inputs['Surface'])
        o.data.materials.append(mat);p.material_index=len(o.data.materials)-1
        for li in p.loop_indices:
            pixel=project(o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co)
            uv.data[li].uv=((pixel[0]-box[0])/width,1-(pixel[1]-box[1])/height)
scene.render.engine='CYCLES';scene.cycles.samples=12;scene.cycles.use_denoising=False
scene.view_settings.view_transform='Standard';scene.render.film_transparent=True
scene.render.resolution_x=256;scene.render.resolution_y=256;scene.render.resolution_percentage=100
points=[o.matrix_world@v.co for o in objects for v in o.data.vertices]
center=Vector(tuple((min(p[k]for p in points)+max(p[k]for p in points))/2 for k in range(3)))
views={}
for i in range(8):
    az=math.radians((i%4)*90);elevation=math.radians(35 if i<4 else 65)
    direction=Vector((math.sin(az)*math.cos(elevation),-math.cos(az)*math.cos(elevation),math.sin(elevation)))
    data=bpy.data.cameras.new(f'View{i}');camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
    data.type='ORTHO';data.ortho_scale=100;data.clip_end=10000
    camera.location=center+direction*1000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();views[str(i)]=camera.name
scene.camera=scene.objects[views['0']]
bpy.context.preferences.filepaths.save_version=0;budget()
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
assert (OUT/'model.blend').stat().st_size<8*1024**2;budget()
report={'status':'PRIVATE_SAVED_CANDIDATE_SOURCE_DIAGNOSTIC_NOT_APPROVED',
        'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),
        'source_plan_sha256':hashlib.sha256(PLAN.read_bytes()).hexdigest(),
        'rightmost_rectangular_variant':best,'pieces':plan['pieces'],'ground_queries':ground_queries,
        'source_diagnostic':{'accepted_pixel_centers':accepted,'model_centers_outside_mask':outside_mask,
                             'mask_centers_without_model':mask_uncovered,
                             'limits':'Mask005 includes foreign ground; rejected mask pixels are not automatically missing wood. Per-face first-hit source diagnostic requires visual ownership review.'},
        'unknown_surfaces':'Neutral gray; no generated textures',
        'ground_context':'Actual saved receiver queried before removal; no synthetic ground baked into asset.'}
(OUT/'candidate.json').write_text(json.dumps(report,indent=2)+'\n')
# Reopen before all material/solid views to inspect the saved file.
bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));scene=bpy.context.scene
render_views(scene.name,views,OUT/'review',modes=('textured','solid'),width=256)
for mode in ('textured','solid'):
    sheet=Image.new('RGBA',(1024,552),(35,40,45,255));draw=ImageDraw.Draw(sheet)
    for i in range(8):
        im=Image.open(OUT/'review'/f'{i}-{mode}.png').convert('RGBA');x=(i%4)*256;y=(i//4)*276
        sheet.alpha_composite(im,(x,y+20));draw.text((x+5,y+4),'Original art camera'if i==0 else f'View{i}',fill='white')
    sheet.convert('RGB').save(OUT/f'{mode}8.png')
budget();print(json.dumps({'output':str(OUT),'ground_samples':len(ground_queries),'model_sha256':report['model_sha256']}))
