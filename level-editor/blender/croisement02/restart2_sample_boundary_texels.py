"""Read saved first-hit UV samples at independently assigned contour pixel centres."""
import argparse,json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--workspace',type=Path);parser.add_argument('--review-directory',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
 for index in ([35] if args.workspace else [35,43,45,46]):
  worker=args.workspace or OUT/f'restart2-wood/projected/assets/croisement02-tree-{index}';review=args.review_directory or OUT/f'restart2-wood/tree{index}-boundary-review-v3';meta=json.loads((review/'evidence.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();mask=np.array(Image.open(meta['boundary_mask']).convert('L'))>0;source=np.array(Image.open(worker/'reference/source.png').convert('RGB'));trees=[];images={};rows=[]
  for obj in bpy.data.collections['Croisement02 Working'].all_objects:
   if obj.type!='MESH' or obj.get('asset_group')!=worker.name or obj.get('projection_component')=='crown':continue
   obj.data.calc_loop_triangles();tris=list(obj.data.loop_triangles);tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(t.vertices) for t in tris],all_triangles=True);trees.append((obj,tris,tree))
  for y,x in zip(*np.nonzero(mask)):
   origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000;hits=[]
   for obj,tris,tree in trees:
    point,normal,face,dist=tree.ray_cast(origin,-RAY)
    if point is not None:hits.append((dist,obj,tris[face],point))
   if not hits:raise ValueError('Geometry gap')
   _,obj,tri,point=min(hits,key=lambda r:r[0]);mat=obj.data.materials[tri.material_index];nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
   if len(nodes)!=1:raise ValueError('Unexpected atlas')
   node=nodes[0];im=node.image;uvnode=node.inputs['Vector'].links[0].from_node;uv=obj.data.uv_layers[uvnode.uv_map];coords=[Vector((*uv.data[i].uv,0)) for i in tri.loops];world=[obj.matrix_world@obj.data.vertices[i].co for i in tri.vertices];p=barycentric_transform(point,*world,*coords);w,h=im.size
   if im.name not in images:
    a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);images[im.name]=np.rint(a.reshape(h,w,4)*255).astype(int)
   a=images[im.name];tx=min(w-1,max(0,int(p.x*w)));ty=min(h-1,max(0,int(p.y*h)));rgb=a[ty,tx,:3];native=source[y,x];neighbors=source[max(0,y-1):y+2,max(0,x-1):x+2,:3];error=int(np.max(np.abs(rgb-native)));near=int(np.min(np.max(np.abs(neighbors.astype(int)-rgb),axis=2)));rows.append(dict(pixel=[int(x),int(y)],object=obj.name,atlas_name=im.name,atlas_pixel=[tx,ty],atlas_rgb=rgb.tolist(),source_rgb=native.tolist(),exact_error=error,neighbor_error=near))
  write_json(review/'texel-audit.json',dict(model_sha256=sha(worker/'model.blend'),target_pixels=len(rows),exact_source_matches=sum(r['exact_error']==0 for r in rows),source_3x3_matches=sum(r['neighbor_error']==0 for r in rows),samples=rows,scope='Nearest saved atlas texel at first wood triangle hit; independent target pixels. Neighbor matching measures subpixel bake sampling only, not exact target equality.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
