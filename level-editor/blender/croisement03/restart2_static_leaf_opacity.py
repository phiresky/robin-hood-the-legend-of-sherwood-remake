"""Inspect added static foliage with original physical alpha and neutral leaf RGB."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main(tree):
 assert tree in (12,14);p=B/f'tree{tree}-static-leaf-crown-v1';out=p/'physical-opacity-audit';out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(p/'worker.blend'));scene=bpy.data.scenes['Croisement03 Refinement'];materials=set();images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data}
  leaves=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement03-arbre06-fragment-tree{tree}-provisional'];assert len(leaves)==2
  for o in leaves:
   for mat in o.data.materials:
    if mat in materials:continue
    materials.add(mat);nodes=mat.node_tree.nodes;mix=next(n for n in nodes if n.type=='MIX_SHADER');assert mix.inputs[0].links[0].from_socket.name=='Alpha';assert mix.inputs[1].links[0].from_node.type=='BSDF_TRANSPARENT';em=mix.inputs[2].links[0].from_node;assert em.type=='EMISSION'
    for link in list(em.inputs[0].links):mat.node_tree.links.remove(link)
    em.inputs[0].default_value=(.55,.55,.55,1)
  assert images=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data}
  render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out,modes=('textured',),width=384);sheet=Image.new('RGB',(1536,768),'#333333')
  for i in range(8):
   im=Image.open(out/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
  sheet.save(out/'sheet.png');write_json(out/'receipt.json',dict(status='Physical alpha diagnostic complete; visual review required',model_sha256=sha(p/'worker.blend'),leaf_objects=[o.name for o in leaves],leaf_materials=[m.name for m in materials],native_view_index=0,alpha_uv_geometry_images_unchanged=True,modified_worker_saved=False,sheet_sha256=sha(out/'sheet.png')))
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
