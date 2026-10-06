"""Continue existing generated material onto occupied hidden texels of the same rig."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from scipy.spatial import cKDTree
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels,array_hash
from restart5_initial_net_restore_native import images
from refinement_review import _tree
from render_multiview_asset import render
from tree_geometry import RAY,SIN
ROOT=OUT/'restart5-initial-nets'

def clipped(poly,axis,bound,above):
 out=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  ia=(a[axis]>=bound)if above else(a[axis]<=bound);ib=(b[axis]>=bound)if above else(b[axis]<=bound)
  if ia:out.append(a)
  if ia!=ib:out.append(a+(b-a)*((bound-a[axis])/(b[axis]-a[axis])))
 return out

def occupancy(obj,uvname,w,h):
 world=np.zeros((h,w,3),np.float64);occupied=np.zeros((h,w),bool);obj.data.calc_loop_triangles();uv=obj.data.uv_layers[uvname]
 for t in obj.data.loop_triangles:
  q=np.array([uv.data[i].uv[:]for i in t.loops])*[w,h];p=np.array([obj.matrix_world@obj.data.vertices[i].co for i in t.vertices]);m=np.column_stack((q[1]-q[0],q[2]-q[0]));det=np.linalg.det(m)
  if abs(det)<1e-12:continue
  inv=np.linalg.inv(m);lo=np.maximum(0,np.floor(q.min(0)).astype(int));hi=np.minimum([w-1,h-1],np.floor(q.max(0)).astype(int))
  for y in range(lo[1],hi[1]+1):
   for x in range(lo[0],hi[0]+1):
    if occupied[y,x]:continue
    poly=list(q.copy())
    for ax,b,up in [(0,x,True),(0,x+1,False),(1,y,True),(1,y+1,False)]:
     poly=clipped(poly,ax,b,up)
     if not poly:break
    if len(poly)<3:continue
    a=np.array(poly);area=abs(np.sum(a[:,0]*np.roll(a[:,1],-1)-a[:,1]*np.roll(a[:,0],-1)))/2
    if area<1e-10:continue
    point=a.mean(0);b=inv@(point-q[0]);world[y,x]=p[0]+b[0]*(p[1]-p[0])+b[1]*(p[2]-p[0]);occupied[y,x]=True
 return occupied,world

def main(key,mode):
 assert mode in ['audit','repair','repair-projected'];assert shutil.disk_usage(OUT).free>25*1024**3
 exp=ROOT/f'texture-fill-v1/profile-{key}/experiment';parent=exp/'native-retained-v2';bake=exp/('bake-v3-identity'if key=='00'else'bake-v2-identity');out=exp/('unseen-audit-v1'if mode=='audit'else ('unseen-complete-v2' if mode=='repair-projected' else 'unseen-complete-v1'));assert not out.exists();out.mkdir();expected='9829d07f5c96531946281a926b81a13dc254501dbb7374e9332743d40fcc9621'if key=='00'else'6dcb9a3c0798b3bfdf3e12500f6d8280461bb65ca23de45263d2b2bfdda8db2e';assert sha(parent/'worker.blend')==expected;bpy.ops.wm.open_mainfile(filepath=str(parent/'worker.blend'));scene=bpy.context.scene;meta=json.loads((exp/'views.json').read_text());names=set(meta['object_names']);before=snapshot(scene,names);uv_before={n:{u.name:array_hash(np.asarray([d.uv[:]for d in u.data]))for u in scene.objects[n].data.uv_layers}for n in names};native={n:images(scene.objects[n].data.materials[0])for n in names};rows=[]
 for rec in json.loads((bake/'layer-0.json').read_text())['objects']:
  obj=scene.objects[rec['object']];provenance=Path(rec['texel_provenance']['path']);assert sha(provenance)==rec['texel_provenance']['sha256'];ownership=np.load(provenance)['ownership'];mat=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'));node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image);uvname=node.inputs['Vector'].links[0].from_node.uv_map;image=node.image;a=pixels(image).copy();h,w=a.shape[:2];occupied,world=occupancy(obj,uvname,w,h);donors=occupied&(ownership==2);targets=occupied&(ownership==0);assert donors.any();dy,dx=np.nonzero(donors);ty,tx=np.nonzero(targets);dist,index=cKDTree(world[donors]).query(world[targets]);limit=32. if obj.name=='Ground camouflage net'else 2.2;valid=dist<=limit;row=dict(object=obj.name,atlas_size=[w,h],occupied_pixels=int(occupied.sum()),generated_occupied_donors=int(donors.sum()),unfilled_occupied_targets=int(targets.sum()),within_bound=int(valid.sum()),distance_limit=limit,maximum_distance=float(dist.max())if len(dist)else 0,median_distance=float(np.median(dist))if len(dist)else 0,protected_source_pixels=int((ownership==1).sum()),remaining_over_bound=int((~valid).sum()));rows.append(row)
  np.savez_compressed(out/(obj.name.replace(' ','-')+'.npz'),occupied=occupied,target_xy=np.column_stack((tx,ty)),donor_xy=np.column_stack((dx[index],dy[index])),world_distance=dist,accepted=valid)
  if mode.startswith('repair'):
   assert valid.all(),row;b=a.copy();b[ty,tx,:3]=a[dy[index],dx[index],:3]
   if mode=='repair-projected' and obj.name=='Ground camouflage net':
    from project_reviewed_texture import _read
    rawpath=exp/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png';raw=_read(rawpath);vh,vw=raw.shape[:2];view=meta['views'][0];inverse=Matrix(view['camera_matrix_world']).inverted();crop=view['crop'];scale=view['ortho_scale'];allobjects=[scene.objects[n]for n in sorted(names)];tree,owners,_=_tree(allobjects);used=[];witness=[]
    for k,(yy,xx) in enumerate(zip(ty,tx)):
     point=Vector(world[yy,xx]);hit,normal,face,distance=tree.ray_cast(point+RAY*6000,-RAY)
     if hit is None or owners[face].name!=obj.name or (hit-point).length>8:continue
     local=inverse@hit;px=crop['left']+(.5+local.x/scale)*crop['width'];py=vh-crop['top']-(.5-local.y/scale)*crop['height'];x0=int(np.floor(px-.5));y0=int(np.floor(py-.5));ax=px-.5-x0;ay=py-.5-y0
     if not(0<=x0<vw-1 and 0<=y0<vh-1):continue
     color=(raw[y0,x0,:3]*(1-ax)+raw[y0,x0+1,:3]*ax)*(1-ay)+(raw[y0+1,x0,:3]*(1-ax)+raw[y0+1,x0+1,:3]*ax)*ay
     if color.max()<.02:continue
     b[yy,xx,:3]=color;used.append(k);witness.append([float(px),float(py),float((hit-point).length)])
    np.savez_compressed(out/'ground-generated-native-projection.npz',target_indices=np.asarray(used),sheet_xy_distance=np.asarray(witness));row.update(projected_raw_generated_donors=len(used),raw_generated_image=str(rawpath),raw_generated_sha256=sha(rawpath),raw_projection_max_world_distance=max((r[2]for r in witness),default=0),raw_projection_scope='Same-object first-hit top surface in own generated native view, reused for inferred underside only; residual edges use bounded same-object generated atlas donors.')
   assert np.array_equal(a[~targets],b[~targets]);assert np.array_equal(a[:,:,3],b[:,:,3]);assert np.array_equal(a[ownership==1],b[ownership==1]);image.pixels.foreach_set(b.ravel());image.update();image.pack();filled_ownership=ownership.copy();filled_ownership[targets]=4;np.savez_compressed(out/(obj.name.replace(' ','-')+'-filled-provenance.npz'),ownership=filled_ownership,occupied=occupied);row['original_protected_and_padding_exact']=True;row['only_occupied_provenance0_rgb_changed']=True;row['filled_rgb_sha256']=array_hash(b)
 assert snapshot(scene,names)==before;assert uv_before=={n:{u.name:array_hash(np.asarray([d.uv[:]for d in u.data]))for u in scene.objects[n].data.uv_layers}for n in names};assert native=={n:images(scene.objects[n].data.materials[0])for n in names};report=dict(parent_model_sha256=expected,scope='Existing generated RGB copied from physically occupied atlas texels to bounded same-object occupied provenance0 only. No native/source texels, padding, alpha, UV or geometry edits.',objects=rows)
 if mode.startswith('repair'):
  bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);digest=sha(out/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));scene=bpy.context.scene;assert snapshot(scene,names)==before;assert uv_before=={n:{u.name:array_hash(np.asarray([d.uv[:]for d in u.data]))for u in scene.objects[n].data.uv_layers}for n in names};assert native=={n:images(scene.objects[n].data.materials[0])for n in names};original=ROOT/f'candidate-v5/profile-{key}';source_report=json.loads((original/'report.json').read_text());source=np.array(Image.open(source_report['source']['source']).convert('RGBA'));observed=np.array(Image.open(original/'observed-source.png').convert('RGBA'));objects=[scene.objects[n]for n in sorted(names)];tree,owners,_=_tree(objects);tris=[]
  for o in objects:o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
  ox,oy=source_report['source']['origin'];count=0;cache={}
  for y,x in np.argwhere(observed[:,:,3]>0):
   p,n,i,d=tree.ray_cast(Vector((ox+x+.5,-(oy+y+.5)/SIN,0))+RAY*6000,-RAY);assert p is not None;o,t=tris[i];material=o.data.materials[t.material_index];nd=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and n.inputs['Vector'].links[0].from_node.uv_map=='Initial native source projection');uv=o.data.uv_layers['Initial native source projection'];q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);a=cache.setdefault(nd.image.name,np.rint(pixels(nd.image)*255).astype(np.uint8));hh,ww=a.shape[:2];rgba=a[max(0,min(hh-1,int(q.y*hh))),max(0,min(ww-1,int(q.x*ww)))];assert np.array_equal(rgba,source[y,x]);count+=1
  report.update(model_sha256=digest,geometry_uv_unchanged=True,original_native_image_packed_bytes_and_RGBA_exact=True,native_sample_count=count,native_exact_RGBA=count,reopened_preservation='PASS');scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(exp/'views.json',out/'actual',width=384);sheet=Image.new('RGBA',(1536,768))
  for i in range(8):sheet.paste(Image.open(out/'actual'/f'view-{i}-textured.png').convert('RGBA'),((i%4)*384,(i//4)*384))
  sheet.save(out/'actual/textured.png');source_report.update(model_sha256=digest,parent_model_sha256=expected);write_json(out/'report.json',source_report);write_json(out/'preservation.json',report)
 write_json(out/'continuation.json',report);print(json.dumps(rows))
if __name__=='__main__':
 acquire()
 try:main(*sys.argv[sys.argv.index('--')+1:])
 finally:release()
