"""Check source reciprocity with saved neighboring trees and render scoped context."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as common
from restart2_tree02_firsthit_v1 import rows_for
from restart2_tree02_shared_ridge_joint_v5 import hit
B=common.B;F=B/'tree01-native-transition-saved-v2';O=F/'neighbor-review-v3';RAY=Vector((0,-common.C,common.S))
def sha(p):
 import hashlib
 return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 global F,O
 if '--folder' in sys.argv:F=Path(sys.argv[sys.argv.index('--folder')+1]).resolve()
 if '--out' in sys.argv:O=Path(sys.argv[sys.argv.index('--out')+1]).resolve()
 assert not O.exists();common.resources();common.acquire(slots=2)
 try:
  common.resources();O.mkdir();model=F/'worker.blend';pins={str(model):sha(model)};bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;own=[o for o in scene.objects if o.type=='MESH' and (o.name.startswith('Tree01 ') or o.name.startswith('Provisional Tree01 wood'))];ownrows=[];textures={}
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in own
  for obj in own:
   obj.data.calc_loop_triangles();tri=list(obj.data.loop_triangles);verts=[obj.matrix_world@v.co for v in obj.data.vertices];ownrows.append((obj,BVHTree.FromPolygons(verts,[list(t.vertices)for t in tri],all_triangles=True),verts,tri))
   for mat in obj.data.materials:
    tex=next((n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'),None)if mat and mat.use_nodes else None
    if tex and tex.image.name not in textures:textures[tex.image.name]=np.array(tex.image.pixels[:]).reshape(tex.image.size[1],tex.image.size[0],4)
  def ownhit(x,y):
   initial=Vector((x+.5,-(y+.5)/common.S,0))+RAY*10000;opaque=[]
   # Traverse each object's alpha independently: advancing one transparent object
   # must not skip a distinct coincident or nearly coincident object's surface.
   for obj,bvh,verts,tri in ownrows:
    origin=initial.copy()
    for step in range(512):
     p,n,f,d=bvh.ray_cast(origin,-RAY)
     if p is None:break
     t=tri[f];mat=obj.data.materials[obj.data.polygons[t.polygon_index].material_index];transparent=mat.use_nodes and any(z.type=='BSDF_TRANSPARENT'for z in mat.node_tree.nodes)
     if mat.get('foliage_card_sides')=='paired-one-sided'and n.dot(RAY)<=0:origin=p-RAY*.002;continue
     tex=next((z for z in mat.node_tree.nodes if z.type=='TEX_IMAGE'),None)if mat.use_nodes else None
     if transparent and tex:
      uv=barycentric_transform(p,*[verts[j]for j in t.vertices],*[Vector((*obj.data.uv_layers.active.data[j].uv,0))for j in t.loops]);im=textures[tex.image.name]
      if not(0<=uv.x<1 and 0<=uv.y<1)or im[int(uv.y*im.shape[0]),int(uv.x*im.shape[1]),3]<.5:origin=p-RAY*.002;continue
     opaque.append([obj.name,t.polygon_index,float((p-initial).length)]);break
    else:raise AssertionError('Own alpha traversal exhausted')
   return min(opaque,key=lambda row:row[2])if opaque else None
  neighbors=[];checks={}
  for number,path,scene_name in [(2,B/'tree02-isolated-prototype-v7/worker.blend','Tree02 isolated'),(3,B/'tree03-ridge-leaf-derivative-v2/worker.blend','Tree13 isolated wood')]:
   pins[str(path)]=sha(path)
   with bpy.data.libraries.load(str(path),link=False)as(src,dst):assert scene_name in src.scenes;dst.scenes=[scene_name]
   imported=dst.scenes[0];bpy.context.window.scene=imported;bpy.context.view_layer.update();objects=[o for o in imported.objects if o.type=='MESH'];rows=rows_for(objects,True);domain=np.array(Image.open(B/f'tree{number:02}-bark-proposal-v1/proposed-bark.png'))>0;leafpath=B/('tree02-isolated-prototype-v7/native-leaves.png'if number==2 else'tree03-canopy-fragment-source-v1/000.png');leaf=np.array(Image.open(leafpath));start=175 if number==2 else 225;domain[:leaf.shape[0],start:start+leaf.shape[1]]|=leaf[:,:,3]>0;changes=[];holes=0
   for y,x in zip(*np.nonzero(domain)):
    baseline=hit(rows,int(x),int(y));intrusion=ownhit(int(x),int(y));holes+=baseline is None
    if baseline and intrusion and intrusion[2]<baseline[2]-1e-3:changes.append([int(x),int(y),baseline,intrusion])
   checks[f'tree{number:02}']={'tested':int(domain.sum()),'baseline_holes':holes,'tree01_intrusions':changes}
   for obj in objects:scene.collection.objects.link(obj);neighbors.append(obj)
  bpy.context.window.scene=scene;bpy.context.view_layer.update();neighborrows=rows_for(neighbors,True);domain=np.zeros((960,1408),bool)
  for name in ['tree01-native-fragment-v1/000.png','tree01-static-leaf-proposal-v1/proposed-static-leaves.png']:
   path=B/name
   if path.exists():
    im=np.array(Image.open(path).convert('RGBA'));domain[:im.shape[0],:im.shape[1]]|=im[:,:,3]>0
  # Use exact cells metadata source path for the static proposal, whose filename is versioned.
  static=json.loads((B/'tree01-native-transition-cpu-v2/cells-static.json').read_text());im=np.array(Image.open(static['source_frame']).convert('RGBA'));domain[:im.shape[0],:im.shape[1]]|=im[:,:,3]>0
  changes=[];holes=0;hole_pixels=[]
  for y,x in zip(*np.nonzero(domain)):
   baseline=ownhit(int(x),int(y));other=hit(neighborrows,int(x),int(y));holes+=baseline is None
   if baseline is None:hole_pixels.append([int(x),int(y)])
   if baseline and other and other[2]<baseline[2]-1e-3:changes.append([int(x),int(y),baseline,other])
  checks['tree01']={'tested':int(domain.sum()),'baseline_holes':holes,'baseline_hole_pixels':hole_pixels,'neighbor_intrusions':changes};report={'status':'PASS_SOURCE_RECIPROCITY'if all(not r.get('tree01_intrusions',r.get('neighbor_intrusions')) and not r['baseline_holes'] for r in checks.values())else'HOLD_SOURCE_INTRUSIONS','pins':pins,'checks':checks,'limits':['Current private Tree01 and pinned approved Tree02/03 geometry; single native shared animation integration remains separate.','No original painted ground included; these are isolated neighboring volumes.']};(O/'firsthits.json').write_text(json.dumps(report,indent=2)+'\n')
  context=[]
  if '--bank' in sys.argv:
   bank=Path(sys.argv[sys.argv.index('--bank')+1]).resolve()/'worker.blend';pins[str(bank)]=sha(bank)
   with bpy.data.libraries.load(str(bank),link=False)as(src,dst):dst.objects=[n for n in src.objects if n.startswith('Candidate bank')]
   assert len(dst.objects)==3
   for obj in dst.objects:scene.collection.objects.link(obj);obj.hide_render=False;context.append(obj)
   report['render_context_only_bank']=str(bank);report['limits'].append('Bank included in uncropped images; this reciprocal tree ray test does not establish bank contact or source ownership.');(O/'firsthits.json').write_text(json.dumps(report,indent=2)+'\n')
  points=[o.matrix_world@v.co for o in own+neighbors+context for v in o.data.vertices];scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=768;scene.render.resolution_y=768;scene.render.resolution_percentage=100;scene.render.film_transparent=True;camera=bpy.data.objects.new('Neighbor review camera',bpy.data.cameras.new('Neighbor review camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.clip_end=10000;scene.camera=camera
  for label,az in [('native',0),('oblique',45)]:
   a=math.radians(az);direction=Vector((math.sin(a)*common.C,-math.cos(a)*common.C,common.S));rot=(-direction).to_track_quat('-Z','Y');origin=Vector(np.array([list(p)for p in points]).mean(0));local=np.array([list(rot.conjugated()@(p-origin))for p in points]);center=(local[:,:2].min(0)+local[:,:2].max(0))/2;target=origin+rot@Vector((*center,0));camera.location=target+direction*5000;camera.rotation_euler=rot.to_euler();camera.data.ortho_scale=max(np.ptp(local[:,0]),np.ptp(local[:,1]))*1.12;scene.render.filepath=str(O/(label+'.png'));bpy.ops.render.render(write_still=True)
  assert all(sha(Path(p))==h for p,h in pins.items());assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<32*1024**2;print(report['status'])
 finally:common.release()
if __name__=='__main__':main()
