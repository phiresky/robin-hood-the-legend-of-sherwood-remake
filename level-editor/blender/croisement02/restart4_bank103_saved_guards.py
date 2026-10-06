"""Check source sampling and shared bank texels on the reopened overlay worker."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY
D=OUT/'restart4-bank103-source-overlay-v2'
def main():
 bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));bpy.context.view_layer.update();o=bpy.data.objects['North Woodland Bank / Native103 contact appearance'];o.data.calc_loop_triangles();verts=[o.matrix_world@v.co for v in o.data.vertices];tris=list(o.data.loop_triangles);tree=BVHTree.FromPolygons(verts,[list(t.vertices)for t in tris],all_triangles=True)
 n=next(n for n in o.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');pixels=np.empty(len(n.image.pixels),np.float32);n.image.pixels.foreach_get(pixels);w,h=n.image.size;rgba=np.rint(pixels.reshape(h,w,4)*255).astype('uint8');crop=np.array(Image.open(D/'native103.png'));assert np.array_equal(rgba[::-1],crop)
 source=np.array(Image.open(OUT/'source-states/covered.png'));prior=json.loads((OUT/'restart4-bank131-current-audit-v2/report.json').read_text());exact=0;excluded=0
 for r in prior['rows']:
  x,y=r['pixel'];point,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000,-RAY)
  if not r['bank_first_hit']:assert point is None;excluded+=1;continue
  assert point is not None;t=tris[i];v=np.array([verts[j]for j in t.vertices]);weights=np.linalg.lstsq(np.column_stack((v[1]-v[0],v[2]-v[0])),np.array(point)-v[0],rcond=None)[0];uv=np.array([o.data.uv_layers.active.data[j].uv[:]for j in t.loops]);coord=np.array([1-weights.sum(),*weights])@uv;ix=int(np.floor(coord[0]*w));iy=int(np.floor(coord[1]*h));assert np.array_equal(rgba[iy,ix],source[y,x]);exact+=1
 bank=bpy.data.objects['North Woodland Bank / North Woodland Bank part 000'];bn=next(n for n in bank.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');bw,bh=bn.image.size
 old=json.loads((OUT/'restart2-bank321/neutral-material-comparison-v1/report.json').read_text());target={tuple(r['pixel'])for r in prior['rows']if r['bank_first_hit']};key=lambda r:(r['image'],int(np.floor(r['uv'][0]*bw)),int(np.floor(r['uv'][1]*bh)))
 targets={key(r)for r in old['rows']if tuple(r['pixel'])in target};shared=[r['pixel']for r in old['rows']if tuple(r['pixel'])not in target and r.get('image')==bn.image.name and key(r)in targets]
 write_json(D/'saved-source-guards.json',dict(status='PASS',model_sha256=sha(D/'model.blend'),packed_crop_exact=True,exact_native_rgba_first_hit_centers=exact,excluded_root_owned_centers_with_no_overlay=excluded,bank_image_size=[bw,bh],bank_interpolation=bn.interpolation,unique_target_bank_texels=len(targets),other_sampled_centers_sharing_target_bank_texels=shared,sharing_limit='Counts cover prior4867 observed bank rays, not every surface point. Linear filtering additionally mixes texels continuously; original atlas was not changed.',original_bank_material_unmodified=True))
 print(exact,excluded,len(targets),len(shared),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
