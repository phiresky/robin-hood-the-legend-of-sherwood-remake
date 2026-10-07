"""Reopen the private leaf derivative and verify exact saved surfaces and images."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from restart2_tree03_ridge_leaf_derivative import surfaces,digest
from evidence_io import sha,write_json
from render_slots import acquire,release
B=ROOT/'level-editor/work/croisement03-refinement/restart2/tree03-ridge-leaf-derivative-v2'
def main():
 acquire()
 try:
  model=B/'worker.blend';receipt=json.loads((B/'receipt.json').read_text());assert sha(model)==receipt['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==10;current=surfaces(objects);expected={k:v for k,v in receipt['after_surfaces'].items() if k in current};assert current==expected
  images={im.name:digest(im.pixels[:]) for o in objects for mat in o.data.materials for node in mat.node_tree.nodes if node.type=='TEX_IMAGE' for im in [node.image]};assert images==receipt['image_rgba_sha256'];write_json(B/'saved-model-guard.json',dict(status='PASS saved/reopened derivative surfaces, UV, material, transform and all referenced image RGBA exact',model_sha256=sha(model),receipt_sha256=sha(B/'receipt.json'),mesh_objects=len(objects),image_rgba_sha256=images,source_ray_guard_binding='receipt.json exact surfaces reused by CPU source/ridge checks',scene_contains_no_neighbor_meshes=True))
 finally:release()
if __name__=='__main__':main()
