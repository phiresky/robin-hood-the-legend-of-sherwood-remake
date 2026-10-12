"""Verify retained fence data and render actual materials for private geometry review."""
import sys,json,hashlib,shutil
from array import array
from pathlib import Path
import bpy
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import _geometry
from render_multiview_asset import render
from refinement_review import _tile
B=R/'level-editor/work/active/croisement02';D=B/'static-terrain-closure-oct11-v1/fence100-tip-candidate-v2'
def main():
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3
  report=json.loads((D/'candidate.json').read_text());bpy.ops.wm.open_mainfile(filepath=report['baseline']);before={o.name:_geometry(o,protect_appearance=True)for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'}
  bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));after={o.name:_geometry(o,protect_appearance=True)for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'}
  assert all(after[k]==v for k,v in before.items());assert set(after)-set(before)=={'Source100 elevated wicker tip'}
  scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=128;scene.render.threads_mode='FIXED';scene.render.threads=2
  manifest=json.loads((D/'close-review/views.json').read_text());manifest.pop('render_object_names',None)
  for view in manifest['views']:view['crop']={'width':384,'height':384}
  (D/'actual-frames.json').write_text(json.dumps(manifest,indent=2)+'\n')
  render(D/'actual-frames.json',D/'actual-close',width=384);buffers=[]
  for i in range(8):
   im=bpy.data.images.load(str(D/'actual-close'/f'view-{i}-textured.png'),check_existing=False);buf=array('f',[0])*len(im.pixels);im.pixels.foreach_get(buf);buffers.append(buf);bpy.data.images.remove(im)
  _tile(buffers,384,384,D/'actual-close/textured.png')
  (D/'preservation.json').write_text(json.dumps(dict(status='PASS',original_objects=len(before),retained_geometry_uv_material_data_exact=True,added_objects=['Source100 elevated wicker tip'],saved_model_sha256=hashlib.sha256((D/'model.blend').read_bytes()).hexdigest(),scope='Geometry candidate only; tip is neutral pending geometry approval and texture work.'),indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
