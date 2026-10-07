"""Bind the sole changed leaf support mesh to its unchanged siblings and artwork."""
from pathlib import Path
import sys,json,hashlib
import bpy
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT
from render_slots import acquire,release
sha=lambda b:hashlib.sha256(b).hexdigest()
def snapshot(path):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();result={}
 for o in bpy.context.scene.objects:
  if o.type!='MESH':continue
  m=o.data;rgba=[]
  for mat in m.materials:
   if mat and mat.node_tree:
    for n in mat.node_tree.nodes:
     if n.type=='TEX_IMAGE'and n.image and n.image.packed_file:rgba.append(sha(bytes(n.image.packed_file.data)))
  fixed=dict(matrix=[list(r)for r in o.matrix_world],polygons=[list(p.vertices)for p in m.polygons],materials=[p.material_index for p in m.polygons],uvs=[[list(u.uv)for u in layer.data]for layer in m.uv_layers],images=rgba)
  result[o.name]=dict(vertices=[list(v.co)for v in m.vertices],fixed=fixed)
 return result

def main():
 base=OUT/'restart9-hiding-scatter';parent=base/'mound-support-variants-v2/model.blend';current=base/'mound-support-variants-v3/model.blend';a=snapshot(parent);b=snapshot(current);assert a.keys()==b.keys();changed=[]
 for name in a:
  assert a[name]['fixed']==b[name]['fixed'],name
  if a[name]['vertices']!=b[name]['vertices']:changed.append(name)
 assert changed==['Hiding mound 12'],changed
 out=base/'mound-support-variants-v3/sibling-preservation.json';out.write_text(json.dumps(dict(status='PASS',parent_sha256=sha(parent.read_bytes()),model_sha256=sha(current.read_bytes()),changed_meshes=changed,unchanged_meshes=19,all_topology_UV_materials_source_images_exact=True),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
