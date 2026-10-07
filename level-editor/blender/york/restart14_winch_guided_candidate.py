"""Save one bounded private guided-entry hypothesis with unchanged reviewed wood."""
import ast, hashlib, json, math, shutil, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-components-source-v2';OUT=WORK/'restart2/winch-guided-entry-candidate-v1'
if OUT.exists():raise FileExistsError(OUT)
def budget():
    used=sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file()) if OUT.exists() else 0
    assert used<20*1024**2
    assert shutil.disk_usage(ROOT).free>10*1024**3+20*1024**2-used
budget()
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageOps,ImageFilter
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));back=Vector((0,-c,s));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py'):
    recipe=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0))
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.context.view_layer.update();old=[o for o in scene.objects if o.name.startswith('Suspended chain link')];iron=old[0].data.materials[0];guard={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in old}
for o in old:bpy.data.objects.remove(o,do_unlink=True)
pose,params,path=guided_route();local=[];surface=[];rx=1.5;wire=.35;perimeter=2*math.pi*rx+4*(3-rx)
for j in range(32):
    p,n=capsule(j*perimeter/32,rx)
    for k in range(8):phi=k*math.tau/8;local.append(p+n*(wire*math.cos(phi))+Vector((0,0,wire*math.sin(phi))))
for j in range(32):
    for k in range(8):surface.append((j*8+k,((j+1)%32)*8+k,((j+1)%32)*8+(k+1)%8,j*8+(k+1)%8))
links=[]
for i in range(params['count']):
    mesh=bpy.data.meshes.new(f'Guided chain link {i:03d}');mesh.from_pydata(local,[],surface);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();o=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(o);o['source_node']='scenery-york-castle-winch';o['asset_group']='york-castle-winch';o['native_patch']='patch-004';o.data.materials.append(iron);links.append(o)
def raytree(objects):
    vv=[];ff=[];owners=[]
    for o in objects:
        off=len(vv);vv.extend(o.matrix_world@v.co for v in o.data.vertices)
        for f in o.data.polygons:ff.append(tuple(off+j for j in f.vertices));owners.append(o.get('native_patch')=='patch-004')
    return BVHTree.FromPolygons(vv,ff),owners
fixed,owners=raytree([o for o in scene.objects if o.type=='MESH' and not o.hide_render and o not in links]);source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');f=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')[44];alpha=Image.open(source/f['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));ha=json.loads((WORK/'restart2/winch-motion-physical-v2/hole-owner-audit-v2.json').read_text())['frames'][44];holes={tuple(h['native_pixel']) for h in ha['holes']};points=[]
for y in range(880,979):
    for x in range(2388,2434):
        xx,yy=x-f['bbox'][0],y-f['bbox'][1];inside=0<=xx<alpha.width and 0<=yy<alpha.height;origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;hit=fixed.ray_cast(origin,-back);points.append((origin,hit[3] if hit[0] is not None else 1e20,hit[2] is not None and owners[hit[2]],inside and alpha.getpixel((xx,yy))>=128,inside and core.getpixel((xx,yy))>=128,(x,y) in holes))
rows=[]
for phase in np.arange(0,7,.5):
    for i,o in enumerate(links):p,r=pose(i*spacing+float(phase)/c,i);o.location=p;o.rotation_euler=r.to_euler()
    bpy.context.view_layer.update();chain,_=raytree(links);counts={'phase_native_pixels':float(phase),'opaque_missed':0,'core_missed':0,'empty_filled':0,'hole_centers_filled':0}
    for origin,depth,other,opaque,core_pixel,hole in points:
        hit=chain.ray_cast(origin,-back);shown=other or(hit[0] is not None and hit[3]<depth-1e-5)
        if opaque and not shown:counts['opaque_missed']+=1;counts['core_missed']+=int(core_pixel)
        if not opaque and shown:counts['empty_filled']+=1;counts['hole_centers_filled']+=int(hole)
    rows.append(counts)
rows.sort(key=lambda r:(r['hole_centers_filled'],r['opaque_missed']+r['empty_filled']+3*r['core_missed']));phase=rows[0]['phase_native_pixels']
for i,o in enumerate(links):p,r=pose(i*spacing+phase/c,i);o.location=p;o.rotation_euler=r.to_euler()
bpy.context.view_layer.update();assert guard=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in guard};budget();OUT.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);assert (OUT/'model.blend').stat().st_size<8*1024**2;budget()
result={'status':'Private hypothesis HOLD pending visual/source/guide review','source_model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'outside_geometry_appearance_exact':len(guard),'path':params,'profile':{'rx':rx,'ry':3,'wire_radius':wire},'final_source_fit':rows,'selected_phase_native_pixels':phase,'limitations':['No physical guide shoe or upper bearing authored; curved entry and return need supporting hardware.','630 sampled body/phase combinations clear exact edge crossings; continuous sweep and full containment not proven.','Upper strand centers held fixed; native animation fit remains unverified.','No chain texture synthesis, animation integration or canonical changes.']};(OUT/'proposal.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'model_bytes':(OUT/'model.blend').stat().st_size,'best_final_fit':rows[0]}))
