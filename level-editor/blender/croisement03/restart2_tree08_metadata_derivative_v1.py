"""Bind a label-only Tree08 derivative to its immutable reviewed geometry."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def value(v):
 if isinstance(v,(str,int,float,bool)) or v is None:return v
 if isinstance(v,bpy.types.ID):return [v.bl_rna.identifier,v.name_full]
 try:return [value(x) for x in v]
 except TypeError:return str(v)
def digest(v):return hashlib.sha256(json.dumps(v,sort_keys=True,separators=(',',':')).encode()).hexdigest()
def material(m):
 nodes=[]
 for n in m.node_tree.nodes:
  attrs={}
  for k in ('operation','blend_type','data_type','interpolation','extension','projection','projection_blend','uv_map','vector_type','convert_from','convert_to','distribution','subsurface_method','clamp_factor','clamp_result'):
   if hasattr(n,k):attrs[k]=value(getattr(n,k))
  nodes.append(dict(name=n.name,type=n.bl_idname,attributes=attrs,image=n.image.name if hasattr(n,'image') and n.image else None,inputs={s.identifier:value(s.default_value) for s in n.inputs if hasattr(s,'default_value')}))
 return dict(name=m.name,diffuse=list(m.diffuse_color),nodes=nodes,links=[(l.from_node.name,l.from_socket.identifier,l.to_node.name,l.to_socket.identifier) for l in m.node_tree.links])
def snapshot(scene):
 objects={};materials={}
 for o in scene.objects:
  if o.type!='MESH':continue
  m=o.data;attrs={}
  for a in m.attributes:
   vals=[]
   for d in a.data:
    vals.append({k:value(getattr(d,k)) for k in ('value','vector','color') if hasattr(d,k)})
   attrs[a.name]=dict(domain=a.domain,type=a.data_type,data=vals)
  surf=dict(vertices=[list(v.co) for v in m.vertices],polygons=[(list(f.vertices),f.material_index,f.use_smooth) for f in m.polygons],uv={u.name:[list(d.uv) for d in u.data] for u in m.uv_layers},attributes=attrs,matrix_world=[list(r) for r in o.matrix_world],matrix_basis=[list(r) for r in o.matrix_basis],parent=o.parent.name if o.parent else None,materials=[m.name for m in m.materials],properties={k:value(o[k]) for k in o.keys() if k!='asset_group'})
  objects[o.name]=digest(surf)
  for mat in m.materials:materials[mat.name]=digest(material(mat))
 images={}
 referenced={n.image for m in bpy.data.materials if m.use_nodes for n in m.node_tree.nodes if hasattr(n,'image') and n.image}
 assert referenced,'Expected referenced source images'
 for i in referenced:
  pixels=np.asarray(i.pixels[:],np.float32);assert pixels.size>0,(i.name,'image data unavailable')
  images[i.name]=dict(rgba=hashlib.sha256(pixels.tobytes()).hexdigest(),size=list(i.size),colorspace=i.colorspace_settings.name,alpha_mode=i.alpha_mode,packed_sha256=hashlib.sha256(bytes(i.packed_file.data)).hexdigest() if i.packed_file else None)
 assert len(images)==len(referenced)
 return dict(objects=objects,materials=materials,images=images)
def main():
 src=B/'tree08-crown-prototype-v1/worker.blend';out=B/'tree08-metadata-derivative-v2';assert sha(src)=='1d778f8c83d61c0d8b11f856db508c1651fe0389876f7b8884aa750be0bfc02c';assert src.stat().st_size<1024**2;out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src));scene=bpy.data.scenes['Tree13 isolated wood'];before=snapshot(scene);assert len(before['objects'])==16;changes=[]
  for o in scene.objects:
   if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-09':changes.append(o.name);o['asset_group']='croisement03-tree-08'
  assert len(changes)==9
  assert all(o.get('asset_group') in {'croisement03-tree-08','croisement03-arbre06-fragment-tree08-provisional'} for o in scene.objects if o.type=='MESH');assert snapshot(scene)==before;dst=out/'worker.blend';bpy.data.libraries.write(str(dst),{scene},fake_user=True,compress=True);assert dst.stat().st_size<1024**2
  bpy.ops.wm.open_mainfile(filepath=str(dst));after=snapshot(bpy.data.scenes['Tree13 isolated wood']);assert after==before;assert sha(src)=='1d778f8c83d61c0d8b11f856db508c1651fe0389876f7b8884aa750be0bfc02c'
  frozen=B/'geometry-round9-tree08-v1';write_json(out/'metadata-normalization.json',dict(status='PASS saved-and-reopened metadata-only derivative; frozen review artifact unchanged',source_model=str(src),source_model_sha256=sha(src),model_sha256=sha(dst),model_bytes=dst.stat().st_size,changed_property='asset_group',old='croisement03-tree-09',new='croisement03-tree-08',changed_objects=changes,all16_geometry_uv_attributes_material_bindings_world_transforms_and_other_properties_exact=True,material_node_links_inputs_and_texture_settings_exact=True,packed_image_rgba_alpha_colorspace_exact=True,before=before,after=after,frozen_review_manifest_sha256=sha(frozen/'review-candidates.json'),frozen_review_evidence_sha256=sha(frozen/'gallery/evidence.json'),limits=['No new geometry or appearance approval. User reviews the immutable original model; export must retain this exact derivation receipt.','No render copied or rerun; source geometry/material/image fingerprints prove the same surface.','Static source, runtime membership and terrain exclusions remain unchanged.']));print('PASS',sha(dst),len(changes),dst.stat().st_size)
 finally:release()
if __name__=='__main__':main()
