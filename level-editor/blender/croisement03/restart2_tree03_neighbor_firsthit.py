"""Check Tree04 accepted bark rays against the private Tree03 and reviewed neighbor."""
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
 version=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'v1';out=B/f'tree03-crown-prototype-{version}';assert not (out/'neighbor-firsthit-baseline.json').exists();acquire()
 try:
  model=out/'worker.blend';context=B/'tree04-crown-prototype-v1/worker.blend';bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Tree13 isolated wood'];owned=[o for o in scene.objects if o.type=='MESH']
  with bpy.data.libraries.load(str(context),link=False) as (a,b):b.objects=list(a.objects)
  neighbors=[o for o in b.objects if o.type=='MESH' and o.get('asset_group') in {'croisement03-tree-04','croisement03-arbre07-fragment-tree04-provisional'}];assert len(neighbors)>1;matrices={o:world(o) for o in neighbors}
  for o in neighbors:o.parent=None;scene.collection.objects.link(o);o.matrix_world=matrices[o]
  scene.view_layers.update();rows=[]
  for o in owned+neighbors:
   m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles);tree=BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True);fol=o.get('asset_group') in {'croisement03-arbre07-fragment-tree04-provisional','croisement03-arbre08-fragment-tree03-provisional'};rgba=None
   if fol:
    im=next(n.image for n in m.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');rgba=np.asarray(im.pixels[:],np.float32).reshape(im.size[1],im.size[0],4)
   rows.append((o,tree,vs,ts,m.uv_layers.active,rgba,fol,o in neighbors))
  native_count=json.loads((out/'receipt.json').read_text())['native_faces'];red=np.array(Image.open(B/'tree03-canopy-fragment-source-v1/000.png'));expected_native=[]
  reserved=np.array(Image.open(B/'tree04-bark-proposal-v1/proposed-bark.png'))>0;counts={};baseline_counts={};violations=[]
  for y,x in zip(*np.nonzero(reserved)):
   red_pixel=red[y,x-225] if 0<=y<175 and 225<=x<435 else np.zeros(4,np.uint8);source_red=red_pixel[3]>0
   if source_red:expected_native.append([int(x),int(y)])
   initial=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000
   owners=[]
   for baseline_only in (True,False):
    origin=initial.copy();owner='no-hit'
    for _ in range(256):
     hits=[]
     for i,(_,t,*rest) in enumerate(rows):
      if baseline_only and not rest[-1]:continue
      p,n,f,d=t.ray_cast(origin,-RAY)
      if p is not None:hits.append((d,i,p,n,f))
     if not hits:break
     _,i,p,n,f=min(hits,key=lambda h:h[0]);o,t,vs,ts,uv,rgba,fol,neighbor=rows[i]
     if fol:
      tri=ts[f];u=barycentric_transform(p,*[vs[j] for j in tri.vertices],*[Vector((*uv.data[j].uv,0)) for j in tri.loops]);inside=0<=u.x<1 and 0<=u.y<1
      if not inside or rgba[min(rgba.shape[0]-1,int(u.y*rgba.shape[0])),min(rgba.shape[1]-1,int(u.x*rgba.shape[1])),3]<.5:origin=p-RAY*.002;continue
     owner='Tree04' if neighbor else 'Tree03-inferred-or-wood'
     if not neighbor and fol and tri.polygon_index<native_count:
      sample=np.rint(rgba[min(rgba.shape[0]-1,int(u.y*rgba.shape[0])),min(rgba.shape[1]-1,int(u.x*rgba.shape[1]))]*255).astype(np.uint8)
      if source_red and np.array_equal(sample,red_pixel):owner='Tree03-native-source'
     break
    owners.append(owner)
   baseline,owner=owners;baseline_counts[baseline]=baseline_counts.get(baseline,0)+1
   counts[owner]=counts.get(owner,0)+1
   if owner!=('Tree03-native-source' if source_red else baseline):violations.append([int(x),int(y),owner,o.name if owner!='no-hit' else None])
  write_json(out/'neighbor-firsthit-baseline.json',dict(status='PASS neighbor first hits match source-supported red canopy occlusion exactly' if not violations else 'HOLD neighbor first-hit regression',model_sha256=sha(model),reviewed_neighbor_sha256=sha(context),reserved_samples=int(reserved.sum()),expected_native_red_source_pixels=expected_native,expected_preserved_neighbor_pixels=int(reserved.sum())-len(expected_native),counts=counts,baseline_counts=baseline_counts,baseline_holes=baseline_counts.get('no-hit',0),violations=violations,limits=['Reviewed Tree04 geometry establishes local accepted bark ray ownership. Exactly three native red source samples may occlude the neighbor. Inferred foliage/wood may not replace any neighbor hit. Full joint morphology remains required.']))
 finally:release()
if __name__=='__main__':main()
