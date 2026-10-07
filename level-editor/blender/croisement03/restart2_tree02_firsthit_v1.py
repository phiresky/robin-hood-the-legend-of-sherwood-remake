"""Saved-model Tree02 bark and retained Tree03 source first-hit guard."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';SIN=math.sin(math.radians(35));RAY=Vector((0,-math.cos(math.radians(35)),SIN))
def rows_for(objects,neighbor):
 rows=[]
 for o in objects:
  m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles);tree=BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True);fol='fragment' in o.get('asset_group','');im=next(n.image for n in m.materials[-1].node_tree.nodes if n.type=='TEX_IMAGE');rgba=np.asarray(im.pixels[:],np.float32).reshape(im.size[1],im.size[0],4);rows.append((o,tree,vs,ts,m.uv_layers.active,rgba,fol,neighbor))
 return rows

def sample(rows,x,y):
 origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000
 for _ in range(256):
  hits=[]
  for i,(o,t,*_) in enumerate(rows):
   p,n,f,d=t.ray_cast(origin,-RAY)
   if p is not None:hits.append((d,i,p,n,f))
  if not hits:return (None,None,None,[0,0,0,0])
  d,i,p,n,f=min(hits,key=lambda h:h[0]);o,t,vs,ts,uv,rgba,fol,neighbor=rows[i];tri=ts[f];u=barycentric_transform(p,*[vs[j] for j in tri.vertices],*[Vector((*uv.data[j].uv,0)) for j in tri.loops]);inside=0<=u.x<1 and 0<=u.y<1;pixel=rgba[int(u.y*rgba.shape[0]),int(u.x*rgba.shape[1])] if inside else np.zeros(4)
  if pixel[3]>.5 and (fol or n.dot(RAY)>0):return(o.name,tri.polygon_index,neighbor,np.rint(pixel*255).astype(np.uint8).tolist())
  if not fol:return(o.name,tri.polygon_index,neighbor,[100,100,100,255])
  origin=p-RAY*.002
 raise AssertionError('transparent ray traversal exhausted')

def main():
 ver=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'v1';out=B/f'tree02-isolated-prototype-{ver}';assert not (out/'firsthit.json').exists();acquire()
 try:
  path=out/'worker.blend';context=B/'tree03-crown-prototype-v2/worker.blend';assert sha(context)=='0d71de988e22119da0a19ac0fdc7f0143a8172657e712fd83889ab8cdeaaaf40';bpy.ops.wm.open_mainfile(filepath=str(path));scene=bpy.data.scenes['Tree02 isolated'];owned=[o for o in scene.objects if o.type=='MESH'];ownrows=rows_for(owned,False)
  with bpy.data.libraries.load(str(context),link=False) as (a,b):b.objects=[n for n in a.objects if n.startswith('Tree03 private stem') or n.startswith('Arbre08 fragment provisional') or n.startswith('Inferred cluster support')]
  neighbors=[o for o in b.objects if o and o.type=='MESH'];assert neighbors
  for o in neighbors:scene.collection.objects.link(o)
  scene.view_layers.update();nr=rows_for(neighbors,True);rows=ownrows+nr
  src=np.array(Image.open(B.parent/'baseline/covered.png').convert('RGBA'));mask=np.array(Image.open(B/'tree02-bark-proposal-v1/proposed-bark.png'))>0;ownleaf=np.array(Image.open(out/'native-leaves.png').convert('RGBA'));otherleaf=np.array(Image.open(B/'tree03-canopy-fragment-source-v1/000.png'));sourcefull=np.array(Image.open(B.parent/'animation-references/animation-07/000.png').convert('RGBA'));changes=[];retained=[];ownpixels=[];counts={'bark':0,'own_leaf':0,'neighbor_leaf':0}
  for y,x in zip(*np.nonzero(mask)):
   exp=src[y,x].tolist();kind='bark'
   if y<175 and 175<=x<225 and ownleaf[y,x-175,3]:exp=ownleaf[y,x-175].tolist();kind='own_leaf'
   if y<175 and 225<=x<435 and otherleaf[y,x-225,3]:exp=otherleaf[y,x-225].tolist();kind='neighbor_leaf'
   got=sample(rows,int(x),int(y));counts[kind]+=1
   if got[3]!=exp or (kind=='neighbor_leaf' and got[2] is not True):changes.append([int(x),int(y),kind,got,exp])
   if kind=='neighbor_leaf':retained.append([int(x),int(y),got[0],got[1],got[3]==exp and got[2] is True])
  assert len(retained)==13
  for y,x in zip(*np.nonzero(ownleaf[:,:,3])):
   got=sample(rows,int(x+175),int(y));exp=ownleaf[y,x].tolist()
   if got[3]!=exp:ownpixels.append([int(x+175),int(y),got,exp])
  # All existing accepted Tree03 samples retain exact object/face/RGBA first hits.
  reserved=np.array(Image.open(B/'tree03-bark-proposal-v1/proposed-bark.png'))>0;reserved[:175,225:435]|=otherleaf[:,:,3]>0;violations=[];baselineholes=0
  for y,x in zip(*np.nonzero(reserved)):
   baseline=sample(nr,int(x),int(y));got=sample(rows,int(x),int(y));baselineholes+=baseline[0] is None
   if got!=baseline:violations.append([int(x),int(y),baseline,got])
  result=dict(status='PASS exact source and neighbor first hits' if not changes and not ownpixels and not violations else 'HOLD first-hit source regression',model_sha256=sha(path),neighbor_sha256=sha(context),accepted_bark_samples=377,counts=counts,bark_changes=changes,retained13Tree03NativeLeafOcclusions=retained,own_leaf_samples=int((ownleaf[:,:,3]>0).sum()),own_leaf_changes=ownpixels,neighbor_reserved_samples=int(reserved.sum()),neighbor_baseline_holes=baselineholes,neighbor_firsthit_changes=violations,limits=['Neighbor loaded read-only in memory; not saved or copied into isolated model.','Native source guard does not approve morphology, terrain or shared animation ownership.'])
  write_json(out/'firsthit.json',result);print(result['status'], 'bark',len(changes),'leaf',len(ownpixels),'neighbor',len(violations))
 finally:release()
if __name__=='__main__':main()
