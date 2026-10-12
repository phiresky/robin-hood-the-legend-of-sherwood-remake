"""Check saved foliage against independent bark samples and unresolved native backdrop windows."""
import sys,json,hashlib,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as common
B=common.B;RAY=Vector((0,-common.C,common.S))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 folder=Path(sys.argv[sys.argv.index('--')+1]).resolve();model=folder/'worker.blend';output=folder/'crown-context-guard-v2.json';assert model.exists()and not output.exists();common.resources();common.acquire(slots=2)
 try:
  common.resources();digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));rows=[]
  for o in bpy.context.scene.objects:
   wood=o.name.startswith('Provisional Tree01 wood');crown=o.name.startswith('Tree01 ')
   if o.type!='MESH'or not(wood or crown):continue
   m=o.data;m.calc_loop_triangles();v=[o.matrix_world@p.co for p in m.vertices];tris=list(m.loop_triangles);tree=BVHTree.FromPolygons(v,[list(t.vertices)for t in tris],all_triangles=True);row={'name':o.name,'wood':wood,'tree':tree,'vertices':v,'triangles':tris}
   if crown:
    image=next(n.image for n in m.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');row.update(uv=m.uv_layers.active,pixels=np.asarray(image.pixels[:],np.float32).reshape(image.size[1],image.size[0],4))
   rows.append(row)
  def hit(row,origin):
   probe=origin.copy()
   for step in range(512):
    p,n,f,dist=row['tree'].ray_cast(probe,-RAY)
    if p is None:return None
    if row['wood']:return (p-origin).length
    if n.dot(RAY)>0:
     t=row['triangles'][f];uv=barycentric_transform(p,*[row['vertices'][j]for j in t.vertices],*[Vector((*row['uv'].data[j].uv,0))for j in t.loops]);pixels=row['pixels']
     if 0<=uv.x<=1 and 0<=uv.y<=1 and pixels[min(int(uv.y*pixels.shape[0]),pixels.shape[0]-1),min(int(uv.x*pixels.shape[1]),pixels.shape[1]-1),3]>.5:return (p-origin).length
    probe=p-RAY*.002
   raise AssertionError('Alpha traversal exhausted')
  mask=np.array(Image.open(B/'tree01-bark-seed-proposal-v1/proposed-visible-bark.png'))>0;bark_bad=[];spine_bad=[];native=np.array(Image.open(B/'tree01-native-fragment-v1/000.png'))[:,:,3]>0;static=np.array(Image.open(B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png'))[:,:,3]>0;phase_leaf_samples=[]
  for y,x in np.argwhere(mask):
   spine=Vector((float(x),-float(y)/common.S,0))+RAY*5000
   if not any(hit(r,spine)is not None for r in rows if r['wood']):spine_bad.append([int(x),int(y)])
   origin=Vector((float(x)+.5,-(float(y)+.5)/common.S,0))+RAY*5000;hits=[(distance,r['wood'],r['name'])for r in rows if(distance:=hit(r,origin))is not None];first=min(hits)if hits else None;leaf=x<175 and y<175 and bool(native[y,x]);static_leaf=x<175 and y<175 and bool(static[y,x])
   expected='Tree01 observed native frame0 provisional cells'if leaf else'Tree01 observed native proposed static samples'if static_leaf else'wood'
   if leaf or static_leaf:phase_leaf_samples.append([int(x),int(y),expected])
   if first is None or (expected=='wood'and not first[1])or(expected!='wood'and first[2]!=expected):bark_bad.append([int(x),int(y),'expected '+expected,first])
  roles=json.loads((B/'tree01-source-role-audit-v1/roles.json').read_text());windows=[];window_bad=[];occupancy=np.array(Image.open(B.parent/'baseline/masks/000124.png'))>0;union=np.array(Image.open(B/'tree01-native-fragment-v1/alpha-union.png'))>0
  for sample in roles['samples']:
   if not sample['id'].startswith('shadow-'):continue
   x0,y0,x1,y1=sample['native_box'];assert not occupancy[y0:y1,x0:x1].any()and not union[y0:y1,x0:x1].any();windows.append(sample['native_box'])
   for y in range(y0,y1):
    for x in range(x0,x1):
     origin=Vector((x+.5,-(y+.5)/common.S,0))+RAY*5000
     for row in rows:
      if not row['wood'] and hit(row,origin)is not None:window_bad.append([x,y,row['name']])
  report={'status':'PASS_SOURCE_CONTEXT_SAMPLES'if not bark_bad and not spine_bad and not window_bad else'HOLD_SOURCE_CONTEXT_OCCLUSION','model_sha256':digest,'bark_samples':int(mask.sum()),'bark_failures':bark_bad,'wood_only_spine_failures':spine_bad,'native_phase_leaf_precedence':phase_leaf_samples,'pixel_sampling':'Half-pixel centers for alpha; original integer geometric spine rays independently retained.','native_backdrop_window_boxes':windows,'backdrop_window_samples':sum((x1-x0)*(y1-y0)for x0,y0,x1,y1 in windows),'backdrop_window_crown_hits':window_bad,'limits':['Dark windows are outside the observed shared animation; physical emptiness or deeper static foliage ownership is not inferred.','Only independently scoped samples, not complete source segmentation or native animation runtime proof.']};output.write_text(json.dumps(report,indent=2)+'\n');assert sha(model)==digest;print({'status':report['status'],'bark_failures':len(bark_bad),'window_hits':len(window_bad)});assert not bark_bad and not spine_bad and not window_bad
 finally:common.release()
if __name__=='__main__':main()
