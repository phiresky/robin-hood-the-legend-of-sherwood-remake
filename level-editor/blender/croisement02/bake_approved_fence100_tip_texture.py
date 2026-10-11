"""Bake generated appearance only onto the approved new rail-tip object."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from bake_reviewed_asset import stage
B=R/'level-editor/work/croisement02-refinement';D=B/'approved-fence100-tip-texture-oct11-v2';E=D/'experiment';G=E/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
def main():
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3;manifest=json.loads((E/'views.json').read_text());manifest['texture_receiver_object_names']=['Source100 elevated wicker tip'];p=E/'views-tip-bake.json';p.write_text(json.dumps(manifest,indent=2)+'\n');bpy.ops.wm.open_mainfile(filepath=manifest['source_blend']);scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.engine='CYCLES';scene.cycles.samples=8
  stage(p,G/'generated-preserved.png',E/'bake-v1',texels_per_unit=8,reconciliation_reference=G/'generated-raw.png')
 finally:release()
if __name__=='__main__':main()
