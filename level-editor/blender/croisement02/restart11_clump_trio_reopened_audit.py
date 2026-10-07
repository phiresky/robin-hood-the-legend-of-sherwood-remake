"""Audit actual opacity and gravity support on the saved limited clump trio."""
from pathlib import Path
import sys,json,hashlib,io
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from leaf_state_scene_context import load_scene
from refinement_review import _tree
from restart11_clump_support_trio import rawtree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart11-hiding-mound/support-trio-v1';r=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==r['model_sha256'];out=worker/'reopened-audit-v1';out.mkdir(exist_ok=False)
 source=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));h,w=rgba.shape[:2];audit=json.loads((OUT/'restart9-hiding-scatter/terrain-receivers-v2/report.json').read_text())
 scene,static,pins,base=load_scene();ground=[o for o in static if o.name.startswith('Croisement02 Terrain')or o.get('asset_group')=='croisement02-north-woodland-bank'];wall=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];terrain=rawtree(ground);withwall=rawtree(ground+wall)
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();records=[]
 for row in r['records']:
  objects=[bpy.data.objects[n]for n in row['objects']];tree,_,_=_tree(objects);triangles=[];images={};union=np.zeros((h,w),int);objectrows=[];support=withwall if row['tag']=='wall'else terrain
  for obj in objects:
   obj.data.calc_loop_triangles();triangles.extend((obj,t)for t in obj.data.loop_triangles)
   image=next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');arr=np.array(Image.open(io.BytesIO(bytes(image.packed_file.data))).convert('RGBA'));assert arr.shape==rgba.shape;mask=arr[:,:,3]>0;assert np.array_equal(arr[mask],rgba[mask]);images[obj.name]=arr;union+=mask
   assert all(bytes(next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE').packed_file.data)==bytes(image.packed_file.data)for mat in obj.data.materials)
   gaps=[];near=[];penetrations=[];tested=0
   for tri in obj.data.loop_triangles:
    vs=[obj.matrix_world@obj.data.vertices[i].co for i in tri.vertices];uvs=[obj.data.uv_layers.active.data[i].uv for i in tri.loops]
    for a,b in [(a/5,b/5)for a in range(6)for b in range(6-a)]:
     c=1-a-b;p=vs[0]*a+vs[1]*b+vs[2]*c;uv=uvs[0]*a+uvs[1]*b+uvs[2]*c;x=int(uv.x*w);y=int((1-uv.y)*h)
     if not(0<=x<w and 0<=y<h and mask[y,x]):continue
     hit,_,_,_=support.ray_cast(Vector((p.x,p.y,2000)),Vector((0,0,-1)))
     if hit is None:raise RuntimeError('Missing vertical support')
     gap=p.z-hit.z;tested+=1;gaps.append(gap)
     if -.01<=gap<=.12:near.append([*p])
     if gap<-.01:penetrations.append(dict(position=[*p],gap=gap))
   objectrows.append(dict(object=obj.name,owned_source_pixels=int(mask.sum()),sampled_opaque_points=tested,minimum_vertical_gap=min(gaps,default=None),maximum_vertical_gap=max(gaps,default=None),near_receiver_samples=len(near),penetration_samples=len(penetrations),worst_penetrations=sorted(penetrations,key=lambda x:x['gap'])[:5],zero_opacity_geometry=not bool(mask.any())))
  assert np.array_equal(union, (rgba[:,:,3]>0).astype(int));authority=next(q for q in audit['records']if q['id']==row['instance']);x0,y0=np.array(authority['display_position'])+authority['initial']['offset'];missing=[];foreign=[];wrong=[];error=0.;tested=0
  for y in range(h):
   for x in range(w):
    p,_,ti,_=tree.ray_cast(Vector((float(x0+x+.5),float(-(y0+y+.5)/SIN),0))+RAY*2000,-RAY)
    if rgba[y,x,3]==0:
     if p is not None:foreign.append([x,y])
     continue
    if p is None:missing.append([x,y]);continue
    obj,t=triangles[ti];q=barycentric_transform(p,*[obj.matrix_world@obj.data.vertices[i].co for i in t.vertices],*[Vector((*obj.data.uv_layers.active.data[i].uv,0))for i in t.loops]);sx=q.x*w;sy=(1-q.y)*h;error=max(error,abs(sx-x-.5),abs(sy-y-.5));assert int(sx)==x and int(sy)==y
    if t.material_index!=0 or not np.array_equal(images[obj.name][int(sy),int(sx)],rgba[y,x]):wrong.append([x,y])
    tested+=1
  assert tested==1011 and not(missing or foreign or wrong) and error<.01
  records.append(dict(tag=row['tag'],instance=row['instance'],native_first_hit_status='PASS',opaque_centers=tested,maximum_texel_error=error,missing=missing,foreign=foreign,wrong=wrong,objects=objectrows))
 (out/'report.json').write_text(json.dumps(dict(status='NATIVE_PASS_SUPPORT_DIAGNOSTIC',model_sha256=sha(model),source_sha256=source['source_sha256'],records=records,scope='Independent reopened packed RGBA/UV/first-hit audit. Vertical opaque-surface samples diagnose terrain contact; not a full support-hull or inter-clump stability proof. Fully transparent geometry is explicitly identified for removal.'),indent=2)+'\n')
 print(json.dumps([dict(tag=x['tag'],objects=x['objects'])for x in records]))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
