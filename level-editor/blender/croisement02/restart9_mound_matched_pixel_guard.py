"""Verify every native opaque pixel against reopened leaf volume UVs and bytes."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=OUT/'restart9-hiding-scatter';worker=root/'mound-support-variants-v2';out=worker/'saved-pixel-guard-v1';out.mkdir(exist_ok=False);rec=json.loads((worker/'validation.json').read_text());source=json.loads((root/'mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));model=worker/'model.blend';assert sha(model)==rec['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();results=[];audit=json.loads((root/'terrain-receivers-v2/report.json').read_text())
 for row in rec['records']:
  obj=bpy.data.objects[row['object']];mesh=obj.data;mesh.calc_loop_triangles();tris=list(mesh.loop_triangles);points=[obj.matrix_world@v.co for v in mesh.vertices];tree=BVHTree.FromPolygons(points,[t.vertices for t in tris],all_triangles=True);image=next(n.image for n in mesh.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');assert hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==source['source_sha256'];instance=next(r for r in audit['records']if r['id']==row['instances'][0]);x0,y0=np.array(instance['display_position'])+instance['initial']['offset'];tested=0;maximum=0.;bad=[]
  for y,x in np.argwhere(rgba[:,:,3]>0):
   point,normal,index,d=tree.ray_cast(Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000,-RAY);assert point is not None;tri=tris[index]
   if tri.material_index!=0:bad.append(dict(pixel=[int(x),int(y)],material=tri.material_index,world=list(point),triangle=index));continue
   uv=[Vector((*mesh.uv_layers.active.data[i].uv,0))for i in tri.loops];q=barycentric_transform(point,*[points[i]for i in tri.vertices],*uv);sx=q.x*52;sy=(1-q.y)*33;maximum=max(maximum,abs(sx-x-.5),abs(sy-y-.5));assert int(sx)==x and int(sy)==y;tested+=1
  assert tested+len(bad)==1011 and maximum<.01;results.append(dict(non_source_first_hits=bad,object=obj.name,instances=row['instances'],native_pixel_centers=tested,maximum_UV_pixel_error=maximum,packed_source_sha256=source['source_sha256']))
 (out/'report.json').write_text(json.dumps(dict(status='HOLD'if any(r['non_source_first_hits']for r in results)else 'PASS',model_sha256=sha(model),records=results,scope='Every opaque native center first-hits source-textured top material at exact source texel. Packed PNG bytes exact. Foreground occlusion and terrain interior contact not covered.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
