"""Check reserved Tree05 bark rays against the complete private Tree06 and coarse neighbor."""
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
def world(o):return world(o.parent)@o.matrix_parent_inverse@o.matrix_basis if o.parent else o.matrix_basis.copy()
def main():
 version=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'v3';out=B/f'tree06-crown-prototype-{version}';assert not (out/'neighbor-firsthit.json').exists();acquire()
 try:
  model=out/'worker.blend';context=B.parent/'croisement03-grouped.blend';bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Tree13 isolated wood'];owned=[o for o in scene.objects if o.type=='MESH']
  with bpy.data.libraries.load(str(context),link=False) as (a,b):b.objects=list(a.objects)
  neighbors=[o for o in b.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-05'];assert len(neighbors)==3;matrices={o:world(o) for o in neighbors}
  for o in neighbors:o.parent=None;scene.collection.objects.link(o);o.matrix_world=matrices[o]
  scene.view_layers.update();rows=[]
  for o in owned+neighbors:
   m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles);tree=BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True);fol=o.get('asset_group')=='croisement03-arbre07-fragment-tree06-provisional';rgba=None
   if fol:
    im=next(n.image for n in m.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');rgba=np.asarray(im.pixels[:],np.float32).reshape(im.size[1],im.size[0],4)
   rows.append((o,tree,vs,ts,m.uv_layers.active,rgba,fol,o in neighbors))
  old=np.array(Image.open(B/'tree06-bark-proposal-v2/proposed-bark.png'))>0;own=np.array(Image.open(B/'tree06-bark-proposal-v3/proposed-bark.png'))>0;reserved=old&~own;counts={};violations=[]
  for y,x in zip(*np.nonzero(reserved)):
   origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000;owner='no-hit'
   for _ in range(256):
    hits=[]
    for i,(_,t,*_) in enumerate(rows):
     p,n,f,d=t.ray_cast(origin,-RAY)
     if p is not None:hits.append((d,i,p,n,f))
    if not hits:break
    _,i,p,n,f=min(hits,key=lambda h:h[0]);o,t,vs,ts,uv,rgba,fol,neighbor=rows[i]
    if fol:
     tri=ts[f];u=barycentric_transform(p,*[vs[j] for j in tri.vertices],*[Vector((*uv.data[j].uv,0)) for j in tri.loops]);inside=0<=u.x<1 and 0<=u.y<1
     if not inside or rgba[min(rgba.shape[0]-1,int(u.y*rgba.shape[0])),min(rgba.shape[1]-1,int(u.x*rgba.shape[1])),3]<.5:origin=p-RAY*.002;continue
    owner='Tree05' if neighbor else 'Tree06';break
   counts[owner]=counts.get(owner,0)+1
   if owner!='Tree05':violations.append([int(x),int(y),owner,o.name if owner!='no-hit' else None])
  write_json(out/'neighbor-firsthit.json',dict(status='PASS all reserved bark first hits remain Tree05' if not violations else 'HOLD reserved neighbor first hits',model_sha256=sha(model),coarse_neighbor_scene_sha256=sha(context),reserved_samples=int(reserved.sum()),counts=counts,violations=violations,limits=['Coarse Tree05 geometry establishes local reserved-ray ownership only; no final neighbor texture or ground-contact completion.']))
 finally:release()
if __name__=='__main__':main()
