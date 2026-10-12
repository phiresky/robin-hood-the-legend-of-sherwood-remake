"""Independently sample saved native UVs and render all eight material views."""
import sys,json,io,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_bank_face_source_saved as source
import restart2_bank_full_v1 as base

def main():
 if '--folder' in sys.argv:source.OUT=Path(sys.argv[sys.argv.index('--folder')+1]).resolve()
 source.guard.resources();source.guard.acquire(slots=2)
 try:
  source.guard.resources();folder=source.OUT;receipt=json.loads((folder/'projection.json').read_text());model=folder/'worker.blend';assert source.sha(model)==receipt['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;rows=[];images={}
  for obj in scene.objects:
   if obj.type!='MESH':continue
   obj.data.calc_loop_triangles();tri=list(obj.data.loop_triangles);verts=[obj.matrix_world@v.co for v in obj.data.vertices];rows.append((obj,BVHTree.FromPolygons(verts,[list(t.vertices)for t in tri],all_triangles=True),tri,verts))
  original=np.array(Image.open(source.B.parent/'baseline/covered.png').convert('RGBA'));fail=[];samples=[p[:2]for p in receipt['protected_firsthits']]+receipt['accepted_pixels']
  for x,y in samples:
   origin=Vector((x+.5,-(y+.5)/base.S,0))+base.RAY*5000;hits=[]
   for obj,bvh,tri,verts in rows:
    p,n,f,d=bvh.ray_cast(origin,-base.RAY)
    if p is not None:hits.append((d,obj.name,p,f,tri,verts))
   d,name,p,f,tri,verts=min(hits,key=lambda h:h[0]);obj=scene.objects[name];t=tri[f];mat=obj.data.materials[obj.data.polygons[t.polygon_index].material_index];tex=next((n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'),None)
   if tex is None:fail.append([x,y,'missing texture']);continue
   if tex.image.name not in images:images[tex.image.name]=np.array(Image.open(io.BytesIO(bytes(tex.image.packed_file.data))).convert('RGBA'))
   rgba=images[tex.image.name];uv=barycentric_transform(p,*[verts[j]for j in t.vertices],*[Vector((*obj.data.uv_layers.active.data[j].uv,0))for j in t.loops]);xx=math.floor(uv.x*rgba.shape[1]);yy=math.floor((1-uv.y)*rgba.shape[0])
   if not(0<=xx<rgba.shape[1]and 0<=yy<rgba.shape[0])or not np.array_equal(rgba[yy,xx],original[y,x]):fail.append([x,y,'RGBA/UV mismatch',xx,yy])
  report={'status':'PASS_EXACT_SAVED_NATIVE_RGBA'if not fail else'HOLD_SOURCE_MISMATCH','model_sha256':source.sha(model),'samples':len(samples),'protected':3446,'new':len(receipt['accepted_pixels']),'failures':fail};(folder/'saved-rgba-guard.json').write_text(json.dumps(report,indent=2)+'\n');assert not fail,fail[:10]
  views={f'view-{i}':f'Bank view{i}'for i in range(8)};base.render_views(scene.name,views,folder/'views',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(folder/'views'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(folder/f'{mode}8.png')
  assert source.sha(model)==receipt['model_sha256'];assert sum(p.stat().st_size for p in folder.rglob('*')if p.is_file())<32*1024**2
 finally:source.guard.release()
if __name__=='__main__':main()
