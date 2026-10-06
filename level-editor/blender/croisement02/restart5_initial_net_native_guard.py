"""Audit saved initial rigging topology and exact observed source first-hit samples."""
import sys,json,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from refinement_review import _tree
from tree_geometry import RAY,SIN
from render_slots import acquire,release
ROOT=OUT/'restart5-initial-nets'
def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 for key in ['00','01']:
  w=ROOT/f'candidate-v2/profile-{key}';report=json.loads((w/'report.json').read_text());model=w/'model.blend';assert sha(model)==report['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();obs=[o for o in bpy.context.scene.objects if o.type=='MESH'];tree,owners,_=_tree(obs);tris=[];topology=[]
  for o in obs:
   bm=bmesh.new();bm.from_mesh(o.data);assert all(e.is_manifold for e in bm.edges);topology.append(dict(object=o.name,closed_volume=bm.calc_volume(),minimum_z=min((o.matrix_world@v.co).z for v in o.data.vertices)));bm.free();o.data.calc_loop_triangles();tris.extend([(o,t)for t in o.data.loop_triangles])
  assert len(tris)==len(owners);rec=report['source'];source=np.array(Image.open(rec['source']).convert('RGBA'));observed=np.array(Image.open(w/'observed-source.png').convert('RGBA'));assert np.array_equal(source[:,:,:3],observed[:,:,:3]);assert np.all(observed[:,:,3]<=source[:,:,3]);yxs=np.argwhere(observed[:,:,3]>0);samples=[];arrays={}
  for y,x in yxs:
   ox,oy=rec['origin'];p,n,i,dist=tree.ray_cast(Vector((ox+x+.5,-(oy+y+.5)/SIN,0))+RAY*6000,-RAY)
   if p is None:samples.append(dict(pixel=[int(x),int(y)],hit=False));continue
   o,t=tris[i];assert owners[i]==o;mat=o.data.materials[t.material_index];nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image]
   if not nodes:samples.append(dict(pixel=[int(x),int(y)],hit=True,observed=False,object=o.name));continue
   node=nodes[0];uvname=node.inputs['Vector'].links[0].from_node.uv_map;uv=o.data.uv_layers[uvname];coords=[Vector((*uv.data[j].uv,0))for j in t.loops];world=[o.matrix_world@o.data.vertices[j].co for j in t.vertices];q=barycentric_transform(p,*world,*coords);image=node.image;ww,hh=image.size
   if image.name not in arrays:
    a=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(a);arrays[image.name]=np.rint(a.reshape(hh,ww,4)*255).astype(np.uint8)
   xx=max(0,min(ww-1,int(q.x*ww)));yy=max(0,min(hh-1,int(q.y*hh)));rgba=arrays[image.name][yy,xx];exact=bool(np.array_equal(rgba,source[y,x]));samples.append(dict(pixel=[int(x),int(y)],hit=True,observed=bool(rgba[3]),exact_rgba=exact,object=o.name))
  write_json(w/'saved-native-guard.json',dict(model_sha256=report['model_sha256'],source_sha256=sha(Path(rec['source'])),observed_source_sha256=sha(w/'observed-source.png'),observed_source_RGB_exact=True,observed_alpha_subset=True,topology=topology,target=len(samples),hits=sum(r['hit']for r in samples),exact_native_RGBA=sum(r.get('exact_rgba',False)for r in samples),ambiguous_native_only=rec['source_roles']['ambiguous_native_only'],samples=samples,limitations=['Center rays are a sampling diagnostic distinct from rendered coverage.','Unknown reverse and inferred continuation are intentionally gray until geometry approval and fill.']))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
