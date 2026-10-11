"""Distinguish genuine unfilled surface texels from unused atlas padding."""
import sys,json,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];O=ROOT/'level-editor/work/york-refinement/restart2'/(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'restart60-winch-chain-texture-bake-v1')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
import bpy,numpy as np,math
from mathutils import Vector
from mathutils.bvhtree import BVHTree
bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));report=json.loads((O/'validation.json').read_text());rows=[];pending=[]
for item in report['layers'][0]['objects']:
 obj=bpy.context.scene.objects[item['object']];obj.data.calc_loop_triangles();owner=np.load(item['texel_provenance']['path'])['ownership'];h,w=owner.shape;domain=np.zeros((h,w),bool);unknown_points={}
 for tri in obj.data.loop_triangles:
  mat=obj.data.materials[obj.data.polygons[tri.polygon_index].material_index];uvnode=next(n for n in mat.node_tree.nodes if n.type=='UVMAP');uv=obj.data.uv_layers[uvnode.uv_map];pts=np.array([list(uv.data[i].uv) for i in tri.loops])*[w,h]
  lo=np.maximum(0,np.floor(pts.min(axis=0)).astype(int));hi=np.minimum([w,h],np.ceil(pts.max(axis=0)).astype(int))
  if np.any(hi<=lo):continue
  yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];p=np.stack([xx+.5,yy+.5],-1);a,b,c=pts;v0=b-a;v1=c-a;den=v0[0]*v1[1]-v1[0]*v0[1]
  if abs(den)<1e-12:continue
  q=p-a;s=(q[:,:,0]*v1[1]-v1[0]*q[:,:,1])/den;t=(v0[0]*q[:,:,1]-q[:,:,0]*v0[1])/den;inside=(s>=-1e-6)&(t>=-1e-6)&(s+t<=1+1e-6);domain[lo[1]:hi[1],lo[0]:hi[0]]|=inside
  local=np.array([list(obj.data.vertices[i].co) for i in tri.vertices]);unfilled=inside&(owner[lo[1]:hi[1],lo[0]:hi[0]]==0)
  for iy,ix in np.argwhere(unfilled):
   xyz=local[0]+s[iy,ix]*(local[1]-local[0])+t[iy,ix]*(local[2]-local[0]);unknown_points[(int(yy[iy,ix]),int(xx[iy,ix]))]=(xyz.tolist(),list(tri.normal))
 pending.extend({'object':obj.name,'uv_pixel':list(key),'local_point':value[0],'local_normal':value[1]} for key,value in unknown_points.items())
 rows.append({'object':obj.name,'surface_texels':int(domain.sum()),'unfilled_surface_texels':int(((owner==0)&domain).sum()),'source_texels':int(((owner==1)&domain).sum()),'generated_texels':int(((owner==2)&domain).sum())})
scene=bpy.context.scene;directions=[]
for elev in (-70,-35,0,35,70):
 for az in range(0,360,30):
  e=math.radians(elev);a=math.radians(az);directions.append(Vector((math.sin(a)*math.cos(e),-math.cos(a)*math.cos(e),math.sin(e))))
for item in pending:item['witnesses']=[]
for frame in range(45):
 scene.frame_set(frame*2);bpy.context.view_layer.update();vv=[];ff=[]
 for o in scene.objects:
  if o.type!='MESH' or o.get('native_patch')!='patch-004':continue
  off=len(vv);vv.extend(o.matrix_world@v.co for v in o.data.vertices);ff.extend(tuple(off+i for i in p.vertices) for p in o.data.polygons)
 tree=BVHTree.FromPolygons(vv,ff)
 for item in pending:
  o=scene.objects[item['object']];p=o.matrix_world@Vector(item['local_point']);normal=(o.matrix_world.to_3x3().inverted().transposed()@Vector(item['local_normal'])).normalized()
  for i,d in enumerate(directions):
   if normal.dot(d)<=.12:continue
   hit=tree.ray_cast(p+d*10000,-d)[0]
   if hit is not None and (hit-p).length<=.02:item['witnesses'].append({'frame':frame,'direction_index':i,'cosine':normal.dot(d),'first_hit_error':(hit-p).length})
counts={str(i):sum(any(w['direction_index']==i and w['frame']==44 for w in p['witnesses']) for p in pending) for i in range(len(directions))}
out={'status':'VISIBILITY_AUDIT','model_sha256':hashlib.sha256((O/'worker.blend').read_bytes()).hexdigest(),'scope':'60 camera directions at all45 source poses; all109 mechanism meshes occlude. No room context. Finite audit, not proof of all-direction invisibility.','unfilled_surface_points':len(pending),'seen_points':sum(bool(p['witnesses']) for p in pending),'directions':[list(d) for d in directions],'frame44_counts':counts,'points':pending};(O/'remaining-visibility.json').write_text(json.dumps(out,indent=2)+'\n');print({'unfilled':len(pending),'seen':out['seen_points'],'best_frame44':sorted(counts.items(),key=lambda x:-x[1])[:8]})
