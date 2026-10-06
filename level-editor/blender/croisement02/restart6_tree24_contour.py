"""Privately correct bounded lower-trunk contours on the approved tree mesh."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,SPECS,OUT,RAY,SIN,COS
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels,array_hash
from refinement_review import _tree
def projection(points):return np.column_stack((points[:,0],-points[:,1]*SIN-points[:,2]*COS))
def projected_tree(objects):
 verts=[];world=[];faces=[];owners=[]
 for obj in objects:
  obj.data.calc_loop_triangles();p=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);q=projection(p);base=len(verts);verts.extend(Vector((x,y,0))for x,y in q);world.extend(Vector(v)for v in p)
  for t in obj.data.loop_triangles:
   ids=tuple(base+j for j in t.vertices);a,b,c=[verts[j]for j in ids]
   if abs((b-a).cross(c-a).z)<1e-5:continue
   faces.append(ids);owners.append(obj.name)
 return BVHTree.FromPolygons(verts,faces,all_triangles=True),verts,world,faces,owners
def covered(objects,targets):
 tree,owners,_=_tree(objects);hits=[]
 for x,y in targets:
  p,n,i,d=tree.ray_cast(Vector((x,-y/SIN,0))+RAY*6000,-RAY)
  hits.append(p is not None and p.z>=-.0001)
 return np.array(hits)
record=next(r for r in json.load(open(OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json'))['records']if r['asset_id']=='croisement02-tree-24')
SPECS[24]=(Path(record['model']),record['model_sha256'])
def main():
 assert shutil.disk_usage(OUT).free>25*2**30;source,digest=SPECS[24];assert sha(source)==digest;out=ROOT/'tree24-contour-v1';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;bpy.context.view_layer.update();asset='croisement02-tree-24';own=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];names={o.name for o in own};original=snapshot(scene,names);wood=[o for o in own if 'Crown'not in o.name];mutable=[o for o in wood if o.get('source_node')in ['building-124']];before={o.name:np.array([o.matrix_world@v.co for v in o.data.vertices])for o in own};uv_before={o.name:{u.name:array_hash(np.array([v.uv[:]for v in u.data]))for u in o.data.uv_layers}for o in own};mask=np.array(Image.open(ROOT/'source-audit-v1/exposed-24.png'))>0;ys,xs=np.where(mask);requested=np.column_stack((xs+.5,ys+.5));inventory=json.loads((OUT/'review-mask-inventory.json').read_text());native_item=next(x for x in inventory['masks']if x['index']==24);nx,ny=native_item['box_top_left'];nmask=np.array(Image.open(native_item['png']))>0;nys,nxs=np.where(nmask);all_native=np.column_stack((nxs+nx+.5,nys+ny+.5));prior_covered=covered(wood,all_native);required=prior_covered|mask[nys+ny,nxs+nx];targets=all_native[required];history=[]
 for iteration in range(12):
  cov=covered(wood,targets);history.append(dict(iteration=iteration,covered=int(cov.sum()),total=len(targets)));print(history[-1],flush=True)
  if cov.all():break
  tree,verts,world,faces,owners=projected_tree(wood);constraints=[]
  for q in targets[~cov]:
   p,n,i,d=tree.find_nearest(Vector((*q,0)));ids=faces[i];anchor=barycentric_transform(p,*[verts[j]for j in ids],*[world[j]for j in ids]);delta=q-np.array(p[:2]);dist=np.linalg.norm(delta)
   if dist<1e-4:continue
   constraints.append((np.array(p[:2]),np.array(anchor),delta*(1+.7/dist),float(d),owners[i]))
  for obj in mutable:
   p=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);q=projection(p);region=(q[:,0]>-20)&(q[:,0]<30)&(q[:,1]>858)&(q[:,1]<896);ids=np.flatnonzero(region);a=p[ids];aq=q[ids];sums=np.zeros((len(ids),2));weights=np.zeros(len(ids));ray=np.array(RAY)
   for center,anchor,delta,dist,owner in constraints:
    if owner!=obj.name:continue
    radius=max(4.,dist*2+3);dq=np.linalg.norm(aq-center,axis=1);depth=abs((a-anchor)@ray);w=np.exp(-2*(dq/radius)**2-2*(depth/(radius+8))**2);w[(dq>radius*1.5)|(depth>radius+14)]=0;sums+=w[:,None]*delta;weights+=w
   shift=sums/(1+weights[:,None]);move=np.column_stack((shift[:,0],-SIN*shift[:,1],-COS*shift[:,1]));a+=move;low=a[:,2]<.15;a[low,1]-=(.15-a[low,2])*COS/SIN;a[low,2]=.15;inverse=obj.matrix_world.inverted()
   for i,point in zip(ids,a):obj.data.vertices[int(i)].co=inverse@Vector(point)
   obj.data.update()
  bpy.context.view_layer.update()
 coverage=covered(wood,targets);movement={};changed={}
 for obj in own:
  p=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);d=np.linalg.norm(p-before[obj.name],axis=1);movement[obj.name]=dict(changed_vertices=int((d>1e-5).sum()),max_world_displacement=float(d.max()),min_world_z=float(p[:,2].min()));changed[obj.name]=d>1e-5
  assert movement[obj.name]['max_world_displacement']<18
  assert uv_before[obj.name]=={u.name:array_hash(np.array([v.uv[:]for v in u.data]))for u in obj.data.uv_layers}
  if obj not in mutable:assert np.array_equal(p,before[obj.name])
 write_json(out/'fit.json',dict(source=str(source),source_sha256=digest,history=history,final_covered=int(coverage.sum()),total=len(targets),movement=movement,missing_pixels=(targets[~coverage]-.5).tolist(),original_covered_required=int(prior_covered.sum()),requested_gap_centers=4,limits=['Only bounded native wood124 edge vertices changed; UVs retained.','Crown and all wood vertices outside the bounded native edge neighborhood remain unchanged.','Private proposal, geometry approval required.']))
 if not coverage.all():return
 # Retain existing filled fallback; exact own native pixels override only
 # changed camera-facing faces, with explicit native UV coordinates.
 inv=json.loads((OUT/'review-mask-inventory.json').read_text());item=next(x for x in inv['masks']if x['index']==24);ox,oy=item['box_top_left'];w,h=item['box_size'];a=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))[oy:oy+h,ox:ox+w].copy();a[:,:,3]=np.array(Image.open(item['png']).convert('L'));Image.fromarray(a).save(out/'native24.png');image=bpy.data.images.load(str(out/'native24.png'));image.pack();materials={};face_counts={};visible_faces={o.name:set()for o in wood};tree,owners,_=_tree(wood);tri_rows=[]
 for o in wood:o.data.calc_loop_triangles();tri_rows.extend((o,t)for t in o.data.loop_triangles)
 for x,y in all_native:
  p,n,i,d=tree.ray_cast(Vector((x,-y/SIN,0))+RAY*6000,-RAY)
  if p is not None and p.z>=-.0001:o,t=tri_rows[i];visible_faces[o.name].add(t.polygon_index)
 for obj in mutable:
  prior_active=obj.data.uv_layers.active.name;uv=obj.data.uv_layers.new(name='Exact lower contour native projection');obj.data.uv_layers.active=obj.data.uv_layers[prior_active];obj.data.uv_layers[prior_active].active_render=True;count=0
  for face in obj.data.polygons:
   for loopid in face.loop_indices:
    p=obj.matrix_world@obj.data.vertices[obj.data.loops[loopid].vertex_index].co;uv.data[loopid].uv=((p.x-ox)/w,1-(-p.y*SIN-p.z*COS-oy)/h)
   if not any(changed[obj.name][i]for i in face.vertices)or face.index not in visible_faces[obj.name]:continue
   slot=face.material_index;key=(obj.name,slot)
   if key not in materials:
    material=obj.data.materials[slot].copy();material.name+=' / lower contour exact native';nodes=material.node_tree.nodes;shader=next((n for n in nodes if n.type in ['EMISSION','BSDF_PRINCIPLED','BSDF_DIFFUSE']),None);socket=shader.inputs['Base Color'if shader.type=='BSDF_PRINCIPLED'else'Color']if shader else next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket if socket.links else None;tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv.name;mix=nodes.new('ShaderNodeMixRGB');material.node_tree.links.new(uvnode.outputs['UV'],tex.inputs['Vector']);material.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);
    if old:material.node_tree.links.new(old,mix.inputs[1])
    else:mix.inputs[1].default_value=socket.default_value
    material.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);material.node_tree.links.new(mix.outputs[0],socket);obj.data.materials.append(material);materials[key]=len(obj.data.materials)-1
   face.material_index=materials[key];count+=1
  face_counts[obj.name]=count
 # Keep a sparse derivative; all other assets remain in their untouched source.
 for obj in list(bpy.data.objects):
  if obj.type=='MESH'and obj.name not in names:bpy.data.objects.remove(obj,do_unlink=True)
 for obj in own:obj.hide_render=False
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'candidate.json',dict(model_sha256=sha(out/'model.blend'),source_sha256=digest,fit_sha256=sha(out/'fit.json'),native_material_faces=face_counts,source_pixels=4,approval='pending private review',scope='Local physical trunk-edge deformation, exact native projection over retained filled fallback.'));assert sha(source)==digest
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
