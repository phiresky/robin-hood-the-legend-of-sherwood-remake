"""Independently sample saved bark UVs and render the unmodified private derivative."""
import sys,json,math,io,argparse
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
sys.path.insert(0,str(Path(__file__).parent))
import restart2_tree01_canopy_saved as common
B=common.B;F=B/'tree01-bark-seed-projection-v2';O=F/'saved-review-v1';RAY=Vector((0,-common.C,common.S))
def main():
 global F,O
 parser=argparse.ArgumentParser();parser.add_argument('--folder',type=Path,default=F);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);F=args.folder;O=F/'saved-review-v1'
 common.resources();common.acquire(slots=2)
 try:
  common.resources();assert not O.exists();O.mkdir();model=F/'worker.blend';digest=common.sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;source=np.array(Image.open(B.parent/'baseline/covered.png').convert('RGBA'));proof=json.loads((B/'tree01-native-transition-saved-v2/bark-receiver-294-v1/receipt.json').read_text());rows=[]
  for obj in scene.objects:
   if not obj.name.startswith('Provisional Tree01 wood'):continue
   m=obj.data;m.calc_loop_triangles();v=[obj.matrix_world@p.co for p in m.vertices];tris=list(m.loop_triangles);rows.append((obj,v,tris,BVHTree.FromPolygons(v,[list(t.vertices)for t in tris],all_triangles=True)))
  projection=json.loads((F/'receipt.json').read_text());assert projection['model_sha256']==digest
  phasepin=projection['full_native_phase_proof'];assert common.sha(Path(phasepin['path']))==phasepin['sha256'];phaseproof=json.loads(Path(phasepin['path']).read_text())
  for path,digest0 in phaseproof['pins'].items():assert common.sha(Path(path))==digest0,path
  excluded={tuple(p)for p in projection['withheld_full_native_foreground_samples']};assert len(excluded)==9
  extra=projection.get('additional_receiver_proof')
  if extra:
   assert common.sha(Path(extra['path']))==extra['sha256'];additional=json.loads(Path(extra['path']).read_text());assert additional['status']=='PASS_SAVED_BARK_RECEIVERS';proof['samples']+=additional['samples']
  wanted={tuple(r['source'])for r in proof['samples']}-excluded
  assigned=[tuple(p)for r in projection['assignments']for p in r['pixels']]
  assert wanted==set(assigned) and len(assigned)==len(wanted)==(432 if extra else 285)
  assignment_faces={(r['object'],r['polygon'])for r in projection['assignments']};textured_faces=set();audited_pixels=[];max_uv_error=0.0
  for obj,_,_,_ in rows:
   for face in obj.data.polygons:
    mat=obj.data.materials[face.material_index]
    if mat.use_nodes and any(n.type=='TEX_IMAGE' for n in mat.node_tree.nodes):textured_faces.add((obj.name,face.index))
  assert textured_faces==assignment_faces,('Unexpected textured wood faces',textured_faces^assignment_faces)
  for row in projection['assignments']:
   obj=scene.objects[row['object']];face=obj.data.polygons[row['polygon']];mat=obj.data.materials[face.material_index];images=[n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'];assert len(images)==1
   packed=bytes(images[0].packed_file.data);assert __import__('hashlib').sha256(packed).hexdigest()==projection['pins'][row['image']]
   rgba=np.array(Image.open(io.BytesIO(packed)).convert('RGBA'));l,t,r,b=row['source_box'];assert rgba.shape==(b-t,r-l,4)
   domain={(int(x+l),int(y+t))for y,x in np.argwhere(rgba[:,:,3]>0)};assert domain=={tuple(p)for p in row['pixels']}
   for x,y in domain:assert np.array_equal(rgba[y-t,x-l],source[y,x]),(x,y,'Complete domain RGBA mismatch')
   audited_pixels.extend(domain)
   for li in face.loop_indices:
    v=obj.matrix_world@obj.data.vertices[obj.data.loops[li].vertex_index].co;uv=obj.data.uv_layers.active.data[li].uv;error=max(abs(uv.x*(r-l)+l-v.x),abs((1-uv.y)*(b-t)+t+v.y*common.S+v.z*common.C));max_uv_error=max(max_uv_error,error)
  assert set(audited_pixels)==wanted and len(audited_pixels)==len(wanted) and max_uv_error<.001
  domain_audit={'status':'PASS','nonzero_alpha_pixels':len(audited_pixels),'textured_faces':len(textured_faces),'unexpected_faces':0,'outside_domain_pixels':0,'duplicate_pixels':0,'maximum_native_uv_error':max_uv_error,'scope':'Every nonzero alpha pixel and every textured wood face; all others remain unassigned.'}
  failures=[];withheld_checks=[]
  for sample in proof['samples']:
   x,y=sample['source'];origin=Vector((x+.5,-(y+.5)/common.S,0))+RAY*5000;hits=[]
   for obj,v,tris,bvh in rows:
    p,n,f,d=bvh.ray_cast(origin,-RAY)
    if p is not None:hits.append((d,obj,v,tris[f],p))
   _,obj,v,tri,p=min(hits,key=lambda h:h[0]);mat=obj.data.materials[obj.data.polygons[tri.polygon_index].material_index];im=next((n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'),None);actual=None
   if im is not None:
    uv=barycentric_transform(p,*[v[i]for i in tri.vertices],*[Vector((*obj.data.uv_layers.active.data[j].uv,0))for j in tri.loops]);rgba=np.array(Image.open(io.BytesIO(bytes(im.packed_file.data))).convert('RGBA'));px=math.floor(uv.x*rgba.shape[1]);py=math.floor((1-uv.y)*rgba.shape[0])
    if 0<=px<rgba.shape[1]and 0<=py<rgba.shape[0]:actual=rgba[py,px]
   if (x,y)in excluded:
    clear=actual is None or actual[3]==0;withheld_checks.append({'source_xy':[x,y],'unassigned':bool(clear)})
    if not clear:failures.append([x,y,'Full native foreground sample still assigned',actual.tolist()])
   elif actual is None or not np.array_equal(actual,source[y,x]):failures.append([x,y,'RGBA mismatch',None if actual is None else actual.tolist()])
  assert not failures,failures[:5]
  own=[o for o in scene.objects if o.type=='MESH' and(o.name.startswith('Tree01 ')or o.name.startswith('Provisional Tree01 wood'))]
  for obj in scene.objects:
   if obj.type=='MESH':obj.hide_render=obj not in own
  scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256;scene.cycles.use_denoising=False;scene.cycles.seed=17;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';cam=bpy.data.objects.new('Bark review camera',bpy.data.cameras.new('Bark review camera'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.sensor_fit='HORIZONTAL';cam.data.clip_end=10000;scene.camera=cam
  def render(name,direction,target,w,h,scale):
   cam.location=target+direction*5000;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=scale;scene.render.resolution_x=w;scene.render.resolution_y=h;scene.render.filepath=str(O/name);bpy.ops.render.render(write_still=True);return Image.open(O/name).convert('RGBA')
  render('native.png',RAY,Vector((85,-105/common.S,0)),260,430,260);points=np.array([list(o.matrix_world@v.co)for o in own for v in o.data.vertices]);center=Vector((points.min(0)+points.max(0))/2);scale=max(np.ptp(points,axis=0))*.92;sheet=Image.new('RGB',(1024,640),'#333333')
  for i in range(8):
   a=math.radians(i*45);im=render(f'actual-{i*45:03}.png',Vector((math.sin(a)*common.C,-math.cos(a)*common.C,common.S)),center,256,320,scale);bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i%4*256,i//4*320))
  sheet.save(O/'actual-eight.png');assert common.sha(model)==digest;(O/'receipt.json').write_text(json.dumps({'status':'PASS_SAVED_RGBA_PENDING_VISUAL_REVIEW','model_sha256':digest,'exact_source_samples':len(wanted),'complete_domain_audit':domain_audit,'withheld_native_checks':withheld_checks,'failures':failures,'images':{p.name:common.sha(p)for p in O.glob('*.png')},'limits':['Sparse reviewed bark spines only; gray unassigned bark remains visible.','Static leaf ownership and final ground/bank context unresolved.']},indent=2)+'\n')
 finally:common.release()
if __name__=='__main__':main()
