"""Retain original shaders on faces straddling the protected upper boundary."""
import sys,hashlib
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree18_contour import SPECS as S18
from restart6_tree39_contour import SPECS as S39,ROOT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main(tree_id):
 source=(S18 if tree_id==18 else S39)[tree_id][0];version=2 if tree_id==18 else 1;parent=ROOT/f'tree{tree_id}-continuous-finished-v{version}/model.blend';out=ROOT/f'tree{tree_id}-continuous-finished-v{version+1}';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));lookup={};names=set()
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or o.get('asset_group')!=f'croisement02-tree-{tree_id}'or'Crown'in o.name:continue
  uv=o.data.uv_layers['Owned source / exterior']
  for f in o.data.polygons:
   mat=o.data.materials[f.material_index];names.add(mat.name)
   for li in f.loop_indices:
    p=o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co
    if p.z>110:lookup.setdefault((tuple(round(x,4)for x in p),tuple(round(x,7)for x in uv.data[li].uv)),set()).add(mat.name)
 bpy.ops.wm.open_mainfile(filepath=str(parent));oldimages={hashlib.sha256(bytes(im.packed_file.data)).hexdigest():im for im in bpy.data.images if im.packed_file}
 with bpy.data.libraries.load(str(source),link=False)as(a,b):b.materials=sorted(names)
 materials=dict(zip(sorted(names),b.materials))
 for mat in materials.values():
  for n in mat.node_tree.nodes:
   if n.type=='TEX_IMAGE'and n.image and n.image.packed_file:
    digest=hashlib.sha256(bytes(n.image.packed_file.data)).hexdigest()
    if digest in oldimages:n.image=oldimages[digest]
 count=0
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or o.get('asset_group')!=f'croisement02-tree-{tree_id}'or'Crown'in o.name:continue
  slots={};uv=o.data.uv_layers['Owned source / exterior']
  for f in o.data.polygons:
   options=None
   for li in f.loop_indices:
    p=o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co
    if p.z<=110:continue
    key=(tuple(round(x,4)for x in p),tuple(round(x,7)for x in uv.data[li].uv));current=lookup[key];options=current if options is None else options&current
   if options is None:continue
   assert len(options)==1,options
   name=next(iter(options))
   if name not in slots:o.data.materials.append(materials[name]);slots[name]=len(o.data.materials)-1
   f.material_index=slots[name];count+=1
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'restoration.json',dict(model_sha256=sha(out/'model.blend'),parent_sha256=sha(parent),approved_parent_sha256=sha(source),upper_faces_restored=count,geometry_unchanged=True,scope='Faces with any original loop above Z110 receive their exact original packed shader graph. Lower native overlays and continuous geometry remain.'))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
