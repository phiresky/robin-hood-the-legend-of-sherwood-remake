"""Reopen the filled tip and verify all retained geometry and fence appearances."""
import sys,json,hashlib,shutil
from pathlib import Path
from array import array
import bpy
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from refinement_workspace import _geometry
from render_slots import acquire,release
from render_multiview_asset import render
from refinement_review import _tile
B=R/'level-editor/work/croisement02-refinement';D=B/'approved-fence100-tip-texture-oct11-v2';E=D/'experiment';BASE=B/'static-terrain-closure-oct11-v1/fence100-tip-candidate-v2'
def main():
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3;bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));geom={o.name:_geometry(o)for o in bpy.data.objects if o.type=='MESH'};outside={o.name:_geometry(o,protect_appearance=True)for o in bpy.data.objects if o.type=='MESH'and o.name!='Source100 elevated wicker tip'}
  bpy.ops.wm.open_mainfile(filepath=str(E/'bake-v1/worker.blend'));assert geom=={o.name:_geometry(o)for o in bpy.data.objects if o.type=='MESH'};assert outside=={o.name:_geometry(o,protect_appearance=True)for o in bpy.data.objects if o.type=='MESH'and o.name!='Source100 elevated wicker tip'}
  scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.threads_mode='FIXED';scene.render.threads=2
  render(BASE/'actual-frames.json',D/'context-actual-v1',width=384);buffers=[]
  for i in range(8):
   im=bpy.data.images.load(str(D/'context-actual-v1'/f'view-{i}-textured.png'),check_existing=False);v=array('f',[0])*len(im.pixels);im.pixels.foreach_get(v);buffers.append(v);bpy.data.images.remove(im)
  _tile(buffers,384,384,D/'context-actual-v1/textured.png');(D/'reopened-preservation.json').write_text(json.dumps(dict(status='PASS',geometry_all_exact=True,retained_objects_geometry_uv_material_exact=len(outside),model_sha256=hashlib.sha256((E/'bake-v1/worker.blend').read_bytes()).hexdigest(),scope='Appearance only on the added tip; no original fence change.'),indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
