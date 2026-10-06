"""Restore five observed grazing foot texels without broad ownership changes."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 parent=ROOT/'tree25-toe-native-v4';guard=json.load(open(parent/'saved-guard.json'));assert guard['model_sha256']==sha(parent/'model.blend');out=ROOT/'tree25-toe-native-v5';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));item=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==25);ox,oy=item['box_top_left'];w,h=item['box_size'];a=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))[oy:oy+h,ox:ox+w].copy();a[:,:,3]=0
 for row in guard['lower_native_failures']:x,y=row['pixel'];a[y-oy,x-ox,3]=255
 Image.fromarray(a).save(out/'native25-grazing5.png');im=bpy.data.images.load(str(out/'native25-grazing5.png'));im.pack();cache={}
 for row in guard['lower_native_failures']:
  o=bpy.data.objects[row['object']];f=o.data.polygons[row['face']];slot=f.material_index;key=(o.name,slot)
  if key not in cache:
   mat=o.data.materials[slot].copy();mat.name+=' / exact five grazing native centers';nodes=mat.node_tree.nodes;socket=next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket;tex=nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';coord=nodes.new('ShaderNodeUVMap');coord.uv_map='Continuous toe native projection';mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(coord.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket);o.data.materials.append(mat);cache[key]=len(o.data.materials)-1
  f.material_index=cache[key]
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'restoration.json',dict(parent_sha256=sha(parent/'model.blend'),model_sha256=sha(out/'model.blend'),guard_sha256=sha(parent/'saved-guard.json'),pixels=[r['pixel']for r in guard['lower_native_failures']],scope='Exact observed RGB overlay on five existing grazing foot rays only; geometry and all previous packed images unchanged.'))
finally:release()
