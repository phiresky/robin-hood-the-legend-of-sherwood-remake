"""Fill only unassigned mound atlas texels from generated or native same-clump material donors."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.kdtree import KDTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import pixels,snapshot
from restart37_state_uv_guard import capture
BASE=OUT/'restart25-approved-state-materialization-v1'

def raster(mesh,layer,w,h):
 positions={};normals={};faces={}
 mesh.calc_loop_triangles()
 for t in mesh.loop_triangles:
  uv=np.array([layer.data[i].uv[:]for i in t.loops],float)*[w,h];v=np.array([mesh.vertices[i].co[:]for i in t.vertices],float);m=np.vstack([uv.T,np.ones(3)])
  if abs(np.linalg.det(m))<1e-12:continue
  inv=np.linalg.inv(m);lo=np.maximum(np.floor(uv.min(axis=0)-.5).astype(int),0);hi=np.minimum(np.ceil(uv.max(axis=0)-.5).astype(int),[w-1,h-1])
  for y in range(lo[1],hi[1]+1):
   for x in range(lo[0],hi[0]+1):
    bary=inv@np.array([x+.5,y+.5,1])
    if bary.min()>=-1e-7:
     key=(y,x);positions[key]=bary@v;normals[key]=np.array(t.normal[:]);faces[key]=t.polygon_index
 return positions,normals,faces

def main():
 assert shutil.disk_usage(OUT).free>=10*1024**3
 assert int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024>=6*1024**3
 prior=BASE/'mound-filled-all-sites-v1/worker.blend';oldproof=json.loads(prior.with_name('preservation.json').read_text());assert sha(prior)==oldproof['model_sha256'];target=BASE/'mound-filled-all-sites-v5';target.mkdir(exist_ok=False)
 authority=json.loads((BASE/'mound-ownership-v1/report.json').read_text());names=[r['object']for site in authority['records']for r in site['objects']];before=capture(prior,names);scene=bpy.context.scene;frozen=snapshot(scene,set(names));layer_report=json.loads((BASE/'official-texture-experiments-v2/mound-initial/experiment/bake-state-v1/layer-0.json').read_text());rows={r['object']:r for r in layer_report['objects']};changed=[];arrays={};original_images={}
 for row in authority['records'][0]['objects']:
  obj=scene.objects[row['object']];mat=next(m for m in obj.data.materials if m.get('source_ownership_bake'));tex=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');image=tex.image;data=pixels(image).copy();h,w=data.shape[:2];prov=rows[obj.name]['texel_provenance'];assert sha(Path(prov['path']))==prov['sha256'];ownership=np.load(prov['path'])['ownership'];assert ownership.shape==(h,w)
  # Provenance arrays use image memory order, matching Blender's bottom-up pixels.
  uvname=tex.inputs['Vector'].links[0].from_node.uv_map;pos,norm,face=raster(obj.data,obj.data.uv_layers[uvname],w,h);donors=[q for q in pos if ownership[q]in(2,3)];needs=[q for q in pos if ownership[q]==0]
  if not needs:continue
  source_donors=False
  if not donors:
   donors=[q for q in pos if ownership[q]==1 and np.ptp(data[q][:3])>.03];source_donors=True
  assert donors,(obj.name,'no generated donors');tree=KDTree(len(donors))
  for i,q in enumerate(donors):tree.insert(Vector(pos[q]),i)
  tree.balance();changes=[]
  for q in needs:
   nearest=tree.find_n(Vector(pos[q]),min(64,len(donors)));eligible=[(dist,i,float(np.dot(norm[q],norm[donors[i]])))for _,i,dist in nearest if np.dot(norm[q],norm[donors[i]])>=.25]
   if not eligible:
    eligible=[(float(np.linalg.norm(pos[q]-pos[d])),i,float(np.dot(norm[q],norm[d])))for i,d in enumerate(donors)if np.dot(norm[q],norm[d])>=.25]
   if not eligible:
    eligible=[(dist,i,float(np.dot(norm[q],norm[donors[i]])))for _,i,dist in nearest]
   dist,i,cos=min(eligible);d=donors[i];data[q][:3]=data[d][:3];changes.append([q[0],q[1],d[0],d[1],dist,cos,face[q],face[d]])
  padding_donors=[q for q in pos if np.ptp(data[q][:3])>.03];padding_tree=KDTree(len(padding_donors))
  for i,q in enumerate(padding_donors):padding_tree.insert(Vector((q[1],q[0],0)),i)
  padding_tree.balance();padding_changes=[]
  for y,x in np.argwhere(ownership==0):
   q=(int(y),int(x))
   if q in pos:continue
   _,i,distance=padding_tree.find(Vector((int(x),int(y),0)))
   if distance<=2.01:
    donor=padding_donors[i];data[q][:3]=data[donor][:3];padding_changes.append([q[0],q[1],donor[0],donor[1],distance])
  old=pixels(image).copy();edited=np.any(old!=data,axis=2);assert not np.any(edited & (ownership!=0));assert np.array_equal(old[:,:,3],data[:,:,3]);original_images[image.name]=dict(old=old,expected=data.copy(),ownership=ownership.copy());image.pixels.foreach_set(data.ravel());image.update();image.pack();arrays[obj.name.replace(' ','_')]=np.array(changes,float);arrays[obj.name.replace(' ','_')+'_padding']=np.array(padding_changes,float);changed.append(dict(object=obj.name,image=image.name,faces=len(obj.data.polygons),unfilled_surface_texels=len(needs),changed_rgb_texels=int(edited.sum()),maximum_donor_distance=max((c[4]for c in changes),default=0),minimum_normal_dot=min((c[5]for c in changes),default=None),unfilled_without_orientation_donor=len(needs)-len(changes),inferred_underside_orientation_fallback_texels=sum(c[5]<.25 for c in changes),bounded_nearest_active_padding_texels=len(padding_changes),source_provenance=prov,donor_scope=('own protected source colors reused only as inferred material' if source_donors else 'own generated material')));print(obj.name,len(needs),int(edited.sum()),flush=True)
 assert snapshot(scene,set(names))==frozen;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(target/'worker.blend'),compress=True);digest=sha(target/'worker.blend');after=capture(target/'worker.blend',names);assert before==after
 for name,r in original_images.items():
  got=pixels(bpy.data.images[name]);assert np.array_equal(np.rint(got*255),np.rint(r['expected']*255)),name;assert np.array_equal(np.rint(got[r['ownership']!=0]*255),np.rint(r['old'][r['ownership']!=0]*255)),name
 np.savez_compressed(target/'changed-texel-donors.npz',**arrays);write_json(target/'preservation.json',dict(status='PASS_INFERRED_ONLY_ATLAS_REPAIR_REVIEW_PENDING',model_sha256=digest,parent_model=str(prior),parent_model_sha256=sha(prior),geometry_native_uv_source_images_exact=True,all_provenance_classes_1_2_3_preserved=True,all_alpha_exact=True,native_centers_preserved_by_identical_geometry_UV_source_images_and_known_shader_branches=20220,repairs=changed,donor_table_sha256=sha(target/'changed-texel-donors.npz'),texture_approval='pending',actual_material_review='pending'))
 print('Saved',digest,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
