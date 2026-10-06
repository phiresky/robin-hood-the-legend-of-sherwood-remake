"""Restore the remaining observed toe texel on an unchanged physical receiver."""
import sys,hashlib,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
from evidence_io import sha,write_json
acquire()
try:
 parent=ROOT/'tree39-continuous-graft-v3/model.blend';out=ROOT/'tree39-continuous-graft-v4';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name];tree,owners,_=_tree(wood);triangles=[]
 for o in wood:o.data.calc_loop_triangles();triangles.extend((o,t)for t in o.data.loop_triangles)
 hit,n,index,d=tree.ray_cast(Vector((1684.5,-663.5/SIN,0))+RAY*6000,-RAY);assert hit is not None;o,tri=triangles[index];face=o.data.polygons[tri.polygon_index];base=o.data.materials[face.material_index];mat=base.copy();mat.name+=' / observed toe restoration';nodes=mat.node_tree.nodes;socket=next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket;image=next(n.image for m in o.data.materials if m and m.use_nodes for n in m.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and n.image.name.startswith('native39'));tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Continuous lower native projection';mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(uv.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket);o.data.materials.append(mat);face.material_index=len(o.data.materials)-1;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'restoration.json',dict(model_sha256=sha(out/'model.blend'),parent_sha256=sha(parent),pixel=[1684,663],object=o.name,face=face.index,geometry_uv_unchanged=True,scope='Original observed toe RGB over retained surface fallback; single receiver material assignment only.'))
finally:release()
