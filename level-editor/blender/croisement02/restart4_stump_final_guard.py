"""Reopen bounded prop corrections and bind preserved payloads/native samples."""
import json,sys,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.geometry import barycentric_transform
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY
from refinement_workspace import _geometry
from refinement_review import _tree

def image_fingerprints():
 result={}
 for image in bpy.data.images:
  if image.type!='IMAGE' or not image.has_data:continue
  a=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(a);result[image.name]=hashlib.sha256(a.tobytes()).hexdigest()
 return result

def main():
 root=OUT/'restart4-source-gaps';specs=[('stump-cap-v5','croisement02-logging-clearing-stumps',OUT/'texture-fill-round-1/croisement02-logging-clearing-stumps/experiment/bake-v1/worker.blend',[13],'building-026')];audit=json.loads((OUT/'restart3-scene-audit/coherent-batch-v3-v1/first-hit/audit.json').read_text());source=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB'))
 for label,asset,parent,regions,changed in specs:
  worker=root/label;proof=json.loads((worker/'proposal.json').read_text());assert sha(parent)==proof['parent_model_sha256'];assert sha(worker/'model.blend')==proof['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.window.scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.view_layer.update();oldobs=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'];preserved={o.name:_geometry(o,True)for o in oldobs if not(o.get('asset_group')==asset and o.get('source_node')==changed)};matrices={o.name:[list(r)for r in o.matrix_world]for o in oldobs};oldimages=image_fingerprints();uvs={o.name:{u.name:[list(x.uv)for x in u.data]for u in o.data.uv_layers}for o in oldobs};bottom={o.name:[list(o.matrix_world@v.co)for v in o.data.vertices[:14]]for o in oldobs if o.get('source_node')==changed}
  bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.window.scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.view_layer.update();obs=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'];byname={o.name:o for o in obs};assert preserved=={n:_geometry(byname[n],True)for n in preserved};assert matrices=={n:[list(r)for r in byname[n].matrix_world]for n in matrices};newimages=image_fingerprints();assert all(newimages.get(n)==h for n,h in oldimages.items());assert all({u:[list(x.uv)for x in byname[n].data.uv_layers[u].data]for u in values}==values for n,values in uvs.items());assert all(max(abs((byname[n].matrix_world@v.co).z-bottom[n][i][2])for i,v in enumerate(byname[n].data.vertices[:14]))<1e-6 for n in bottom)
  own=[o for o in obs if o.get('asset_group')==asset];tree,owners,_=_tree(own);tris=[]
  for o in own:o.data.calc_loop_triangles();tris.extend([(o,t)for t in o.data.loop_triangles])
  assert len(tris)==len(owners);images={};samples=[]
  for region in regions:
   for row in audit['components'][region-1]['samples']:
    x,y=row['pixel'];point,normal,index,distance=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
    if point is None:samples.append(dict(pixel=[x,y],hit=False));continue
    obj,tri=tris[index];assert obj==owners[index];mat=obj.data.materials[tri.material_index];nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and Path(n.image.filepath).name=='native-domain.png']
    if not nodes:samples.append(dict(pixel=[x,y],hit=True,native_overlay=False,object=obj.name));continue
    node=nodes[0];uvname=node.inputs['Vector'].links[0].from_node.uv_map;uv=obj.data.uv_layers[uvname];coords=[Vector((*uv.data[i].uv,0))for i in tri.loops];world=[obj.matrix_world@obj.data.vertices[i].co for i in tri.vertices];q=barycentric_transform(point,*world,*coords);image=node.image;w,h=image.size
    if image.name not in images:
     arr=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(arr);images[image.name]=np.rint(arr.reshape(h,w,4)*255).astype(int)
    ix=max(0,min(w-1,int(q.x*w)));iy=max(0,min(h-1,int(q.y*h)));rgba=images[image.name][iy,ix];samples.append(dict(pixel=[x,y],hit=True,native_overlay=True,object=obj.name,atlas_pixel=[ix,iy],rgba=rgba.tolist(),native=source[y,x].tolist(),exact_rgb=bool(np.array_equal(rgba[:3],source[y,x])),accepted=bool(rgba[3]>0)))
  accepted=[r for r in samples if r.get('accepted')];assert all(r['exact_rgb']for r in accepted)
  write_json(worker/'saved-guard.json',dict(status='PASS',model_sha256=proof['model_sha256'],parent_model_sha256=proof['parent_model_sha256'],preserved_object_count=len(preserved),preserved_objects=list(preserved),original_world_matrices_exact=True,all_original_uv_layers_exact=True,all_original_image_payloads_exact=True,old_image_hashes=oldimages,stump_bottom_ring_exact=False,stump_ground_height_exact=True,samples=samples,native_samples=dict(target=len(samples),hits=sum(r['hit']for r in samples),accepted=len(accepted),accepted_exact_rgb=len(accepted),explicitly_excluded=sum(r.get('native_overlay',False)and not r.get('accepted',False)for r in samples)),limitations=['Native source-facing overlay samples are evaluated independently from preserved old atlas UV.','Physical coverage and accepted RGB are distinct; excluded foreground tree47 remains its own receiver.','Material payload preservation does not inherit any pending parent texture approval.']))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
