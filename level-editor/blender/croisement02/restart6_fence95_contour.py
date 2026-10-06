"""Test a bounded source-supported post foot without changing other fence parts."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_tree39_contour import ROOT,OUT,SPECS,RAY,SIN,COS,projection,projected_tree,covered
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_review import _tree
def face_digest(obj,exclude=()):
 rows=[]
 for p in obj.data.polygons:
  if any(v in exclude for v in p.vertices):continue
  loops=[]
  for i in p.loop_indices:loops.append([list(obj.data.vertices[obj.data.loops[i].vertex_index].co),[list(u.data[i].uv)for u in obj.data.uv_layers]])
  rows.append([p.material_index,sorted(loops)])
 return hashlib.sha256(json.dumps(sorted(rows),separators=(',',':')).encode()).hexdigest()
def main():
 assert shutil.disk_usage(OUT).free>25*2**30;source,digest=SPECS[95];assert sha(source)==digest;out=ROOT/'fence95-contour-v1';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;bpy.context.view_layer.update();asset='croisement02-east-upright-rail-fence-95';objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];main=scene.objects['East upright rail fence 95'];component=next(c for c in json.loads((ROOT/'components-95-v2.json').read_text())['components']if c['component']==14);ids=set(component['vertices']);outside=face_digest(main,ids);cap=[o for o in objects if o!=main][0];cap_before=face_digest(cap);mask=np.array(Image.open(ROOT/'source-audit-v1/exposed-95.png'))>0;ys,xs=np.where(mask);requested=np.column_stack((xs+.5,ys+.5));inventory=json.loads((OUT/'review-mask-inventory.json').read_text());item=next(x for x in inventory['masks']if x['index']==95);ox,oy=item['box_top_left'];w,h=item['box_size'];native_mask=np.array(Image.open(item['png']))>0;ny,nx=np.where(native_mask);all_native=np.column_stack((nx+ox+.5,ny+oy+.5));oldcov=covered(objects,all_native);required=oldcov|mask[ny+oy,nx+ox];targets=all_native[required]
 post=main.copy();post.data=main.data.copy();post.name='Source-supported second-post lower contour';scene.collection.objects.link(post);post['projection_component']='bounded-second-post-foot';bm=bmesh.new();bm.from_mesh(post.data);bm.verts.ensure_lookup_table();bmesh.ops.delete(bm,geom=[v for v in bm.verts if v.index not in ids],context='VERTS');bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=5,use_grid_fill=True);bm.to_mesh(post.data);bm.free();post.data.update();bm=bmesh.new();bm.from_mesh(main.data);bm.verts.ensure_lookup_table();bmesh.ops.delete(bm,geom=[bm.verts[i]for i in ids],context='VERTS');bm.to_mesh(main.data);bm.free();main.data.update();assert face_digest(main)==outside;assert face_digest(cap)==cap_before;objects.append(post);before=np.array([post.matrix_world@v.co for v in post.data.vertices]);history=[]
 for iteration in range(14):
  cov=covered(objects,targets);history.append(dict(iteration=iteration,covered=int(cov.sum()),required=len(targets)));print(history[-1],flush=True)
  if cov.all():break
  tree,verts,world,faces,owners=projected_tree([post]);p=np.array([post.matrix_world@v.co for v in post.data.vertices]);q=projection(p);sums=np.zeros((len(p),2));weights=np.zeros(len(p));ray=np.array(RAY)
  for target in targets[~cov]:
   nearest,n,i,d=tree.find_nearest(Vector((*target,0)));ids2=faces[i];anchor=np.array(barycentric_transform(nearest,*[verts[j]for j in ids2],*[world[j]for j in ids2]));delta=target-np.array(nearest[:2]);dist=np.linalg.norm(delta)
   if dist<1e-4:delta=np.array([0,.3]);dist=.3
   else:delta*=1+.65/dist
   radius=max(4.,dist*2+3);dq=np.linalg.norm(q-np.array(nearest[:2]),axis=1);depth=abs((p-anchor)@ray);weight=np.exp(-2*(dq/radius)**2-2*(depth/(radius+6))**2);weight[(dq>radius*1.5)|(depth>radius+10)|(q[:,1]<767)]=0;sums+=weight[:,None]*delta;weights+=weight
  shift=sums/np.maximum(1,weights[:,None]);p+=np.column_stack((shift[:,0],-SIN*shift[:,1],-COS*shift[:,1]));low=(p[:,2]<.15)&(q[:,1]>=767);p[low,1]-=(.15-p[low,2])*COS/SIN;p[low,2]=.15;inverse=post.matrix_world.inverted()
  for v,point in zip(post.data.vertices,p):v.co=inverse@Vector(point)
  post.data.update();bpy.context.view_layer.update()
 cov=covered(objects,targets);newcov=covered(objects,all_native);p=np.array([post.matrix_world@v.co for v in post.data.vertices]);change=np.linalg.norm(p-before,axis=1);write_json(out/'fit.json',dict(source_sha256=digest,history=history,covered=int(cov.sum()),required=len(targets),requested_covered=int(covered(objects,requested).sum()),lost_native=int((oldcov&~newcov).sum()),max_world_displacement=float(change.max()),min_post_z=float(p[:,2].min()),other_fence_geometry_uv_exact=face_digest(main)==outside and face_digest(cap)==cap_before,notes=['Original post upper section retained; lower source contour and uncertain mottled foot inferred from native fence mask.','All other rails/posts/cap remain exact.','Near-ground .15 offset avoids coplanar source sampling; this is not a floor ownership transfer.']))
 if not cov.all():return
 assert change.max()<12;assert np.all(change[projection(before)[:,1]<767]==0);assert face_digest(main)==outside;assert face_digest(cap)==cap_before
 a=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))[oy:oy+h,ox:ox+w].copy();a[:,:,3]=np.array(Image.open(item['png']));Image.fromarray(a).save(out/'native95.png');image=bpy.data.images.load(str(out/'native95.png'));image.pack();tree,owners,_=_tree(objects);tris=[]
 for o in objects:o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
 visible=set()
 for x,y in all_native:
  hit,n,i,d=tree.ray_cast(Vector((x,-y/SIN,0))+RAY*6000,-RAY)
  if hit is not None and tris[i][0]==post:visible.add(tris[i][1].polygon_index)
 prior=post.data.uv_layers.active.name;uv=post.data.uv_layers.new(name='Exact lower post native projection');post.data.uv_layers.active=post.data.uv_layers[prior];slots={}
 for face in post.data.polygons:
  for li in face.loop_indices:
   point=post.matrix_world@post.data.vertices[post.data.loops[li].vertex_index].co;uv.data[li].uv=((point.x-ox)/w,1-(-point.y*SIN-point.z*COS-oy)/h)
  if face.index not in visible or not any(change[i]>1e-5 for i in face.vertices):continue
  slot=face.material_index
  if slot not in slots:
   material=post.data.materials[slot].copy();material.name+=' / exact lower post native';nodes=material.node_tree.nodes;shader=next((n for n in nodes if n.type in ['EMISSION','BSDF_PRINCIPLED','BSDF_DIFFUSE']),None);socket=shader.inputs['Base Color'if shader.type=='BSDF_PRINCIPLED'else'Color']if shader else next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket if socket.links else None;tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv.name;mix=nodes.new('ShaderNodeMixRGB');material.node_tree.links.new(uvnode.outputs['UV'],tex.inputs['Vector']);material.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);material.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);material.node_tree.links.new(mix.outputs[0],socket)
   if old:material.node_tree.links.new(old,mix.inputs[1])
   else:mix.inputs[1].default_value=socket.default_value
   post.data.materials.append(material);slots[slot]=len(post.data.materials)-1
  face.material_index=slots[slot]
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'candidate.json',dict(model_sha256=sha(out/'model.blend'),source_sha256=digest,fit_sha256=sha(out/'fit.json'),approval='pending private review'));assert sha(source)==digest
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
