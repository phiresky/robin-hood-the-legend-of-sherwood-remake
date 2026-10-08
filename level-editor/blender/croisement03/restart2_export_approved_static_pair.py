"""Private export of approved wood plus both distinct crown source roles."""
import hashlib,io,json,shutil,struct,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).resolve().parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from evidence_io import sha,write_json
from render_slots import acquire,release
from export_editor import export_editor
from restart2_adjacent_export_composite_v2 import flatten_normal_gate,finalize_document
from restart2_static_pair_export_guard import validate
B=R/'level-editor/work/croisement03-refinement/restart2'
def mesh_signature(obj):
 m=obj.data;return (tuple(tuple(v.co) for v in m.vertices),tuple(tuple(p.vertices) for p in m.polygons),tuple((u.name,tuple(tuple(x.uv) for x in u.data)) for u in m.uv_layers),tuple(tuple(r) for r in obj.matrix_world))
def crown(leaf,role,native_faces,asset,collection):
 before=mesh_signature(leaf);m=leaf.data;assert len(m.materials)==1 and len(m.polygons)>=native_faces
 material=m.materials[0];nodes=[n for n in material.node_tree.nodes if n.type=='TEX_IMAGE'];assert len(nodes)==1;image=nodes[0].image;assert image.packed_file;rgba=np.array(Image.open(io.BytesIO(bytes(image.packed_file.data))).convert('RGBA'))
 if role=='static-native-samples':assert len(m.polygons)==native_faces
 m.calc_loop_triangles();native_triangles=sum(t.polygon_index<native_faces for t in m.loop_triangles);triangles=len(m.loop_triangles)
 if leaf.name not in collection.objects:collection.objects.link(leaf)
 leaf['asset_group']=asset;leaf['source_node']=f'foliage-{asset}-{role}';leaf['part_name']=role;leaf['crown_source_role']=role
 new=material.copy();new.name=f'{asset} {role} physical foliage';new.node_tree.nodes.clear();n=new.node_tree.nodes;l=new.node_tree.links;out=n.new('ShaderNodeOutputMaterial');bsdf=n.new('ShaderNodeBsdfPrincipled');bsdf.inputs['Emission Strength'].default_value=0;tex=n.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';l.new(tex.outputs['Color'],bsdf.inputs['Base Color']);l.new(tex.outputs['Alpha'],bsdf.inputs['Alpha']);l.new(bsdf.outputs[0],out.inputs[0]);m.materials[0]=new
 for k,v in dict(private_foliage_alpha=True,crown_source_role=role,foliage_physical_opacity=True,opacity_semantics='physical-coverage',source_ownership_semantics='separate-mask',source_ownership_channel='vertex-color-r',source_ownership_backface='inferred',foliage_backface_fill='source-derived',foliage_unlit=True).items():new[k]=v
 ownership=m.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
 for face in m.polygons:
  for li in face.loop_indices:ownership.data[li].color=(1 if face.index<native_faces else 0,1,1,1)
 m.color_attributes.active_color_index=list(m.color_attributes).index(ownership);m.color_attributes.render_color_index=m.color_attributes.active_color_index
 assert mesh_signature(leaf)==before,'Crown geometry, transforms or UV changed';return dict(object=leaf.name,role=role,triangles=triangles,native_triangles=native_triangles,rgba_sha256=hashlib.sha256(rgba.tobytes()).hexdigest(),geometry_uv_exact=True)
def main(tree):
 assert tree in (12,14);recipepath=B/'approved-hub-textures-v1/static-pair-staging-v2/source-guards-and-recipes.json';recipe=json.loads(recipepath.read_text())
 for p,d in recipe['pins'].items():assert sha(Path(p))==d,p
 entry=next(r for r in recipe['rows'] if r['asset_id']==f'croisement03-tree-{tree}');source=Path(entry['approved_model']);out=B/f'approved-hub-textures-v1/static-pair-export-v1/tree{tree}';assert not out.exists();assert shutil.disk_usage(R).free>=10*1024**3;available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['Croisement03 Refinement'];bpy.context.window.scene=scene;scene.render.threads_mode='FIXED';scene.render.threads=2;collection=bpy.data.collections['Croisement03 Working'];asset=entry['asset_id'];wood=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];assert len(wood)==(3 if tree==12 else 2)
  leaves=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==entry['recipe']['foliage_group']];assert len(leaves)==2,'Both approved crown objects required';static=next(o for o in leaves if o.name==entry['recipe']['static_object']);dynamic=next(o for o in leaves if o!=static);assert 'STATIC' in static['source_role']
  images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};records=[flatten_normal_gate(o) for o in wood];crowns=[crown(dynamic,'dynamic-frame0-provenance',entry['recipe']['native_old_foliage_faces'],asset,collection),crown(static,'static-native-samples',entry['static_pixels'],asset,collection)]
  for o in wood+leaves:o['asset_name']=f'North tree{tree}';o['part_name']=o.get('part_name',o.name);o.hide_render=False
  assert images=={name:hashlib.sha256(np.asarray(bpy.data.images[name].pixels[:],np.float32).tobytes()).hexdigest() for name in images}
  out.mkdir(parents=True);write_json(out/'views.json',[dict(camera_matrix=[list(r) for r in bpy.data.objects[f'Tree13 view{i}'].matrix_world],ortho_scale=bpy.data.objects[f'Tree13 view{i}'].data.ortho_scale) for i in range(8)]);export=export_editor('Croisement03',out/'model.glb',asset_id=asset)
  raw=(out/'model.glb').read_bytes();length,kind=struct.unpack_from('<II',raw,12);doc=json.loads(raw[20:20+length]);finalize_document(doc)
  for mat in doc['materials']:
   if mat.get('extras',{}).get('private_foliage_alpha'):mat.update(alphaMode='MASK',alphaCutoff=.5,doubleSided=True)
  encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);tail=raw[20+length:];(out/'model.glb').write_bytes(struct.pack('<III',0x46546c67,2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),kind)+encoded+tail)
  proof=validate(out/'model.glb',crowns,records);assert sha(source)==entry['approved_model_sha256'];assert (out/'model.glb').stat().st_size<=8*1024**2;assert sum(p.stat().st_size for p in out.rglob('*') if p.is_file())<=32*1024**2
  write_json(out/'report.json',dict(status='PRIVATE export content guard PASS; native/surface/runtime visual proof pending',source_sha256=sha(source),model_sha256=sha(out/'model.glb'),source_recipe_sha256=sha(recipepath),crowns=crowns,records=records,export=export,content_guard=proof,limits=['No animation ownership or canonical publication claim.','Saved export native first-hit, wood surface parity and all eight runtime views remain required.','Ground appearance remains separate; no texture generation performed.']))
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
