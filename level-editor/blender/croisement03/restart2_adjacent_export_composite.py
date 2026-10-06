"""Represent the approved normal-gated dual-UV bark without RGB resampling."""
import sys,math,hashlib,json,struct
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
from export_editor import export_editor
B=ROOT/'level-editor/work/croisement03-refinement/restart2';TREE=int(sys.argv[sys.argv.index('--')+1]);assert TREE in (12,14)
CONVERTER=ROOT/'level-editor/work/croisement02-refinement/restart2-textures/exact_composite_export_v2.py'
assert sha(CONVERTER)=='5457f186ef9e34b4665a6db941caaeecf61a583dd341947e25f29dc41bc389d2'
sys.path.insert(0,str(CONVERTER.parent))
from exact_composite_export_v2 import convert,finalize_document
RAY=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))))

def clip(poly,front):
 result=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  inside=lambda v:v['dot']>=0 if front else v['dot']<=0
  if inside(a):result.append(a)
  if inside(a)!=inside(b):
   t=a['dot']/(a['dot']-b['dot']);result.append(dict(p=a['p'].lerp(b['p'],t),n=a['n'].lerp(b['n'],t),dot=0.,uv={k:a['uv'][k].lerp(b['uv'][k],t) for k in a['uv']}))
 return result

def flatten_normal_gate(obj):
 old=obj.data;old.calc_loop_triangles();normal_matrix=obj.matrix_world.to_3x3().inverted().transposed();corners=list(old.corner_normals);materials={};vertices=[];uvs={l.name:[] for l in old.uv_layers};normals=[];faces=[];slots=[];cuts=0
 for tri in old.loop_triangles:
  material=old.materials[tri.material_index];mix=next(n for n in material.node_tree.nodes if n.type=='MIX_SHADER');colors=[]
  for socket in (mix.inputs[1],mix.inputs[2]):
   emit=socket.links[0].from_node;assert emit.type=='EMISSION';tex=emit.inputs['Color'].links[0].from_node;assert tex.type=='TEX_IMAGE';colors.append(tex)
  poly=[dict(p=old.vertices[old.loops[li].vertex_index].co.copy(),n=corners[li].vector.copy(),dot=(normal_matrix@corners[li].vector).dot(RAY),uv={l.name:l.data[li].uv.copy() for l in old.uv_layers}) for li in tri.loops]
  if min(v['dot'] for v in poly)<0<max(v['dot'] for v in poly):cuts+=1
  for front in (False,True):
   section=clip(poly,front)
   if len(section)<3:continue
   key=(tri.material_index,front)
   if key not in materials:
    mat=bpy.data.materials.new(material.name+(' / front composite' if front else ' / rear inferred'));mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();links=mat.node_tree.links;out=n.new('ShaderNodeOutputMaterial');textures=[]
    for src in colors:
     t=n.new('ShaderNodeTexImage');t.image=src.image;t.interpolation=src.interpolation;t.extension=src.extension;u=n.new('ShaderNodeUVMap');u.uv_map=src.inputs['Vector'].links[0].from_node.uv_map;links.new(u.outputs['UV'],t.inputs['Vector']);textures.append(t)
    if front:
     m=n.new('ShaderNodeMixRGB');m.blend_type='MIX';links.new(textures[1].outputs['Alpha'],m.inputs[0]);links.new(textures[0].outputs['Color'],m.inputs[1]);links.new(textures[1].outputs['Color'],m.inputs[2]);links.new(m.outputs[0],out.inputs[0])
    else:links.new(textures[0].outputs['Color'],out.inputs[0])
    materials[key]=mat
   for i in range(1,len(section)-1):
    piece=[section[0],section[i],section[i+1]]
    if (piece[1]['p']-piece[0]['p']).cross(piece[2]['p']-piece[0]['p']).length<1e-9:continue
    ids=[]
    for v in piece:
     ids.append(len(vertices));vertices.append(tuple(v['p']));normals.append(tuple(v['n'].normalized()))
     for k in uvs:uvs[k].append(tuple(v['uv'][k]))
    faces.append(ids);slots.append(key)
 mesh=bpy.data.meshes.new(old.name+' normal-gate partition');mesh.from_pydata(vertices,[],faces);mesh.update()
 for k,values in uvs.items():layer=mesh.uv_layers.new(name=k);layer.data.foreach_set('uv',np.asarray(values,np.float32).ravel())
 keys=list(materials)
 for m in materials.values():mesh.materials.append(m)
 for f,key in zip(mesh.polygons,slots):f.material_index=keys.index(key);f.use_smooth=True
 mesh.normals_split_custom_set(normals);obj.data=mesh
 return dict(object=obj.name,original_triangles=len(old.loop_triangles),partition_triangles=len(faces),normal_zero_crossing_triangles=cuts,physical_surface='Only barycentric subdivision; no intentional position offset',complementary=convert(obj))

def main():
 source=B/f'tree{TREE}-approved-wood-texture-v1/source-restored-fill-v1/worker.blend';decision=json.loads((B/f'user-approval-v15/croisement03-tree-{TREE}-shared-crown-fragment.json').read_text());assert sha(source)==decision['model_sha256'];out=B/f'tree{TREE}-exact-export-v1';assert not out.exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['Croisement03 Refinement'];bpy.context.window.scene=scene;collection=bpy.data.collections['Croisement03 Working'];wood=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement03-tree-{TREE}'];assert len(wood)==(3 if TREE==12 else 2);records=[flatten_normal_gate(o) for o in wood]
  leaf=next(o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement03-arbre06-fragment-tree{TREE}-provisional');collection.objects.link(leaf);leaf['asset_group']=f'croisement03-tree-{TREE}';leaf['source_node']=f'foliage-croisement03-tree{TREE}-arbre06-provisional';leaf['part_name']='Provisional Arbre06 crown fragment; dynamic membership unresolved'
  for o in wood+[leaf]:o['asset_name']=f'North tree{TREE}';o['part_name']=o.get('part_name',o.name);o.hide_render=False
  # Export only the private physical alpha/RGB; no runtime membership claim.
  material=leaf.data.materials[0];tex=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE');image=tex.image;new=material.copy();new.node_tree.nodes.clear();n=new.node_tree.nodes;l=new.node_tree.links;outnode=n.new('ShaderNodeOutputMaterial');bsdf=n.new('ShaderNodeBsdfPrincipled');bsdf.inputs['Base Color'].default_value=(0,0,0,1);bsdf.inputs['Emission Strength'].default_value=1;t=n.new('ShaderNodeTexImage');t.image=image;t.interpolation='Closest';t.extension='CLIP';l.new(t.outputs['Color'],bsdf.inputs['Emission Color']);l.new(t.outputs['Alpha'],bsdf.inputs['Alpha']);l.new(bsdf.outputs[0],outnode.inputs[0]);new['private_foliage_alpha']=True;leaf.data.materials[0]=new
  l.remove(bsdf.inputs['Emission Color'].links[0]);bsdf.inputs['Emission Strength'].default_value=0;l.new(t.outputs['Color'],bsdf.inputs['Base Color'])
  for key,value in {'foliage_physical_opacity':True,'opacity_semantics':'physical-coverage','source_ownership_semantics':'separate-mask','source_ownership_channel':'vertex-color-r','source_ownership_backface':'inferred','foliage_backface_fill':'source-derived','foliage_unlit':True}.items():new[key]=value
  native_faces=json.loads((B/f'tree{TREE}-crownfragment-v{4 if TREE==12 else 2}/receipt.json').read_text())['native_faces'];assert native_faces==(950 if TREE==12 else 434)
  ownership=leaf.data.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
  for face in leaf.data.polygons:
   for li in face.loop_indices:ownership.data[li].color=(1 if face.index<native_faces else 0,1,1,1)
  leaf.data.color_attributes.active_color_index=list(leaf.data.color_attributes).index(ownership);leaf.data.color_attributes.render_color_index=leaf.data.color_attributes.active_color_index

  out.mkdir();write_json(out/'views.json',[dict(camera_matrix=[list(row) for row in bpy.data.objects[f'Tree13 view{i}'].matrix_world],ortho_scale=bpy.data.objects[f'Tree13 view{i}'].data.ortho_scale) for i in range(8)]);report=export_editor('Croisement03',out/'model.glb',asset_id=f'croisement03-tree-{TREE}');raw=(out/'model.glb').read_bytes();length,kind=struct.unpack_from('<II',raw,12);doc=json.loads(raw[20:20+length]);finalize_document(doc)
  for mat in doc['materials']:
   if mat.get('extras',{}).get('private_foliage_alpha'):mat.update(alphaMode='MASK',alphaCutoff=.5,doubleSided=True)
  tail=raw[20+length:];encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);(out/'model.glb').write_bytes(struct.pack('<III',0x46546c67,2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),kind)+encoded+tail)
  write_json(out/'report.json',dict(status='PRIVATE diagnostic export; RGB/surface/native/runtime proof pending',source_sha256=sha(source),model_sha256=sha(out/'model.glb'),records=records,export=report,limits=['No publication metadata or dynamic foliage ownership approved.','Complementary coincident MASK branches must be reviewed in runtime; Cycles is not equivalent.','Foliage COLOR0 records original native-facing source cells versus inferred crossed/offmap clusters; alpha remains physical coverage; dynamic membership remains provisional.']))
 finally:release()
if __name__=='__main__':main()
