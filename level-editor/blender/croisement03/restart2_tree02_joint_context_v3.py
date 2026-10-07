"""Selected Tree01/02/03 context, baseline ownership and cropped ground contact."""
import sys,math,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_tree02_firsthit_v1 import rows_for
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
def world(o):return world(o.parent)@o.matrix_parent_inverse@o.matrix_basis if o.parent else o.matrix_basis.copy()
def hit(rows,x,y):
 initial=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000;origin=initial.copy()
 for _ in range(256):
  hits=[]
  for i,(o,t,*_) in enumerate(rows):
   p,n,f,d=t.ray_cast(origin,-RAY)
   if p is not None:hits.append((d,i,p,n,f))
  if not hits:return None
  _,i,p,n,f=min(hits,key=lambda h:h[0]);o,t,vs,ts,uv,rgba,fol,neighbor=rows[i];tri=ts[f]
  if fol:
   u=barycentric_transform(p,*[vs[j] for j in tri.vertices],*[Vector((*uv.data[j].uv,0)) for j in tri.loops])
   if not (0<=u.x<1 and 0<=u.y<1) or rgba[int(u.y*rgba.shape[0]),int(u.x*rgba.shape[1]),3]<.5:origin=p-RAY*.002;continue
  return [o.name,tri.polygon_index,float((p-initial).length)]
 raise AssertionError('alpha traversal exhausted')
def main():
 assert shutil.disk_usage(ROOT).free>10*1024**3+32*1024**2;out=B/'tree02-joint-context-v3';out.mkdir(exist_ok=False);model=B/'tree02-isolated-prototype-v7/worker.blend';other=B/'tree03-crown-prototype-v2/worker.blend';base=B.parent/'baseline/croisement03-baseline.blend';sources={str(p):sha(p) for p in (model,other,base)};acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.data.scenes['Tree02 isolated'];bpy.context.window.scene=s;s.render.threads_mode='FIXED';s.render.threads=2;own=[o for o in s.objects if o.type=='MESH'];orows=rows_for(own,False)
  with bpy.data.libraries.load(str(other),link=False) as (a,b):b.objects=[n for n in a.objects if n.startswith('Tree03 private stem') or n.startswith('Arbre08 fragment provisional') or n.startswith('Inferred cluster support')]
  trees3=[o for o in b.objects if o and o.type=='MESH'];assert len(trees3)==4+len(json.loads((other.parent/'receipt.json').read_text())['inferred_lower_supports'])==10
  for o in trees3:s.collection.objects.link(o)
  selected=[f'building-{n:03}.001' for n in range(2,6)]+['ground.001']
  with bpy.data.libraries.load(str(base),link=False) as (a,b):
   assert all(n in a.objects for n in selected);b.objects=list(selected)
  imported=[o for o in b.objects if o];matrices={o:world(o) for o in imported}
  for o in imported:o.parent=None;o.matrix_world=matrices[o];o.hide_render=False;o.hide_viewport=False;s.collection.objects.link(o)
  tree1=[o for o in imported if o.get('source_node','').startswith('building-')];assert len(tree1)==4;g=next(o for o in imported if o.get('source_node')=='ground');bpy.context.view_layer.update();nrows=rows_for(trees3,True);coarse=[]
  for o in tree1:
   m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles);coarse.append((o,BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True),vs,ts,None,None,False,True))
  level=json.loads((B.parent/'baseline/Croisement03.rhp.json').read_text());rec=level['masks'][1];mask=np.zeros((960,1408),bool);im=np.array(Image.open(B.parent/'baseline/masks/000001.png'));x0,y0=rec['box_top_left'];mask[y0:y0+im.shape[0],x0:x0+im.shape[1]]=im>0;tested=0;holes=0;changes=[]
  for y,x in zip(*np.nonzero(mask)):
   before=hit(coarse+nrows,int(x),int(y));after=hit(coarse+nrows+orows,int(x),int(y));tested+=1
   if before is None:holes+=1;continue
   if after is None or before[:2]!=after[:2] or abs(before[2]-after[2])>.001:changes.append([int(x),int(y),before,after])
  write_json(out/'tree01-ownership.json',dict(status='PASS existing coarse receiver hits unchanged' if not changes else 'HOLD new blocking of existing receiver',source_hashes=sources,mask=1,tested_samples=tested,baseline_holes=holes,changes=changes,limits=['Native mask1 is not classified as pure bark. This conservative check preserves every existing coarse receiver hit without assigning mixed pixels to Tree01.','Baseline holes are disclosed; no claim that unrefined Tree01 geometry or texture is complete.']))
  assert not changes,changes[:8]
  # Restrict the actual ground receiver to a local quad while retaining its own material/UV.
  gm=g.data;gm.calc_loop_triangles();originalvs=[g.matrix_world@v.co for v in gm.vertices];tri=gm.loop_triangles[0];old_uv=gm.uv_layers.active;coords=[];uvs=[]
  for x,y in [(40,-615),(475,-615),(475,-190),(40,-190)]:
   a,b,c=[originalvs[j] for j in tri.vertices];normal=(b-a).cross(c-a);z=a.z-(normal.x*(x-a.x)+normal.y*(y-a.y))/normal.z;p=Vector((x,y,z));coords.append(tuple(p));u=barycentric_transform(p,a,b,c,*[Vector((*old_uv.data[j].uv,0)) for j in tri.loops]);uvs.append(tuple(u[:2]))
  mesh=bpy.data.meshes.new('Local original ground receiver');mesh.from_pydata(coords,[],[(0,1,2,3)]);mesh.update();uv=mesh.uv_layers.new(name='UVMap')
  for i,t in enumerate(uvs):uv.data[i].uv=t
  for mat in gm.materials:mesh.materials.append(mat)
  g.data=mesh;g.matrix_world.identity();g.name='Cropped original ground receiver';groundtree=BVHTree.FromPolygons([Vector(p) for p in coords],[(0,1,2,3)]);main=next(o for o in own if o.get('component')==6);contacts=[]
  for v in list(main.data.vertices)[:20]:
   p=main.matrix_world@v.co;q,n,f,d=groundtree.ray_cast(Vector((p.x,p.y,10)),Vector((0,0,-1)));assert q is not None;contacts.append(float(p.z-q.z))
  assert min(contacts)<=.1 and max(contacts)<.15
  write_json(out/'ground-contact.json',dict(status='PASS lower closure intersects existing local flat receiver within0.1 units; native ledge remains unresolved',source_hashes=sources,ground_crop_world=coords,tree02_base_ring_signed_distances=contacts,minimum=min(contacts),maximum=max(contacts),limits=['Actual baseline ground material shown; painted foliage/ledge remains visible and unfinished.','Source foot near y166 is above this coarse receiver; existing closure/base source238 is not proof of native ledge contact.','No blanket full-terrain or whole-map approval.']))
  target=Vector((244,-410,205));points=[o.matrix_world@v.co for o in s.objects if o.type=='MESH' for v in o.data.vertices];views={};depths={}
  for i in range(8):
   cam=s.objects[f'Tree02 view{i}'];a=i*math.tau/8;direction=Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN));cam.location=target+direction*1500;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=680;bpy.context.view_layer.update();inv=cam.matrix_world.inverted();ds=[-(inv@p).z for p in points];cam.data.clip_start=max(.1,min(ds)-25);cam.data.clip_end=max(ds)+25;assert all(cam.data.clip_start<d<cam.data.clip_end for d in ds);assert all(abs((inv@p).x)<340 and abs((inv@p).y)<340 for p in points),(i,'context outside image');depths[i]=[cam.data.clip_start,cam.data.clip_end];views[f'view-{i}']=cam.name
  render_views(s.name,views,out/'views',modes=('textured',),width=384);sheet=Image.new('RGB',(1536,768),'#333333')
  for i in range(8):
   im=Image.open(out/'views'/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
  sheet.save(out/'sheet.png');assert all(sha(Path(p))==h for p,h in sources.items());size=sum(p.stat().st_size for p in out.rglob('*') if p.is_file());assert size<32*1024**2;write_json(out/'receipt.json',dict(status='Selected context rendered; independent visual review required',source_hashes=sources,tree01_nodes=selected[:-1],tree03_objects=len(trees3),native_view_index=0,camera_depth_bounds=depths,sheet_sha256=sha(out/'sheet.png'),bytes=size,no_blend_or_scene_copy_saved=True,limits=['Coarse Tree01 and painted original ground are context only.','Tree03 exact geometry preserved; user approval pending.','Native ledge receiver and complete shared canopy/runtime ownership remain unresolved.']))
 finally:release()
if __name__=='__main__':main()
