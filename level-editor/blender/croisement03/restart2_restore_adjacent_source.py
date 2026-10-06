"""Apply generated wood only behind the approved original source shader mask."""
import sys,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import _geometry
from render_views import render_views
from evidence_io import sha,write_json
TREE=int(sys.argv[sys.argv.index('--')+1]);assert TREE in (12,14);B=ROOT/f'level-editor/work/croisement03-refinement/restart2/tree{TREE}-approved-wood-texture-v1'
def digest(im):return hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest()
def shape(o):return ([tuple(v.co) for v in o.data.vertices],[tuple(f.vertices) for f in o.data.polygons],tuple(x for row in o.matrix_world for x in row))
def main():
 out=B/'source-restored-fill-v1';assert not out.exists();acquire()
 try:
  baked=B/'packet-v1/experiment/baked-projection-v1/worker.blend';bpy.ops.wm.open_mainfile(filepath=str(baked));data={}
  for o in bpy.context.scene.objects:
   if o.type!='MESH' or o.get('asset_group')!=f'croisement03-tree-{TREE}':continue
   used={f.material_index for f in o.data.polygons};assert len(used)==1
   mat=o.data.materials[used.pop()];tex=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');uvnode=next(n for n in mat.node_tree.nodes if n.type=='UVMAP');im=tex.image
   data[o.name]=dict(shape=shape(o),uv=[tuple(x.uv) for x in o.data.uv_layers[uvnode.uv_map].data],size=tuple(im.size),pixels=np.asarray(im.pixels[:],np.float32),colorspace=im.colorspace_settings.name)
  assert len(data)==(3 if TREE==12 else 2)
  bpy.ops.wm.open_mainfile(filepath=str(B/'normalized.blend'));scene=bpy.data.scenes['Croisement03 Refinement'];bpy.context.window.scene=scene
  originals={im.name:digest(im) for im in bpy.data.images if im.has_data};crown={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.type=='MESH' and o.name not in data};proof=[]
  for name,row in data.items():
   o=bpy.data.objects[name];assert shape(o)==row['shape'];old_uv={u.name:[tuple(x.uv) for x in u.data] for u in o.data.uv_layers};geom=_geometry(o);uv=o.data.uv_layers.new(name='Inferred wood fill');
   for item,value in zip(uv.data,row['uv']):item.uv=value
   im=bpy.data.images.new(name+' inferred wood RGB',width=row['size'][0],height=row['size'][1],alpha=True);im.colorspace_settings.name=row['colorspace'];im.pixels.foreach_set(row['pixels']);im.update();im.pack();slots={}
   for face in o.data.polygons:
    old=face.material_index
    if old not in slots:
     mat=o.data.materials[old].copy();mat.name+=' / inferred bark';nodes=mat.node_tree.nodes;links=mat.node_tree.links;mix=next(n for n in nodes if n.type=='MIX_SHADER');assert mix.inputs[1].links[0].from_node.type=='BSDF_PRINCIPLED';assert mix.inputs[2].links[0].from_node.type=='EMISSION'
     tex=nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Linear';u=nodes.new('ShaderNodeUVMap');u.uv_map=uv.name;emit=nodes.new('ShaderNodeEmission');emit.inputs['Strength'].default_value=1;links.new(u.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],emit.inputs['Color']);links.new(emit.outputs[0],mix.inputs[1]);slots[old]=len(o.data.materials);o.data.materials.append(mat)
    face.material_index=slots[old]
   assert geom==_geometry(o);assert old_uv=={u.name:[tuple(x.uv) for x in u.data] for u in o.data.uv_layers if u.name in old_uv};proof.append(dict(object=name,original_uv_exact=True,geometry_exact=True,original_known_shader_retained=True))
  assert originals=={im.name:digest(im) for im in bpy.data.images if im.name in originals};assert crown=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in crown}
  out.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
  write_json(out/'transfer.json',dict(status='PASS structural transfer; native and visual review pending',model_sha256=sha(out/'worker.blend'),baked_model_sha256=sha(baked),source_model_sha256=sha(B/'normalized.blend'),original_images_exact=True,crown_exact=True,objects=proof))
  render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured',),width=384)
  sheet=Image.new('RGB',(1536,768),'#333333')
  for i in range(8):
   p=Image.open(out/'actual'/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
  sheet.save(out/'actual/textured.png')
 finally:release()
if __name__=='__main__':main()
