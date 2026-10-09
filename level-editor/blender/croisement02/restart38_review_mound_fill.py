"""Review the repaired mound material in its template cameras and exposed site16."""
import sys,json,math
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render
from restart4_stump_final_contact import frame,sheet
from tree_geometry import RAY,SIN,COS
BASE=OUT/'restart25-approved-state-materialization-v1'
def main():
 worker=BASE/'mound-filled-all-sites-v5';model=worker/'worker.blend';proof=json.loads((worker/'preservation.json').read_text());assert sha(model)==proof['model_sha256'];out=worker/'isolated-review-v1';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;meta=json.loads((BASE/'official-texture-experiments-v2/mound-initial/experiment/views.json').read_text());meta['scene_name']=scene.name;meta.pop('render_object_names',None);meta.pop('collection_name',None)
 for ob in scene.objects:
  if ob.type=='MESH':
   ob.hide_render=ob.name not in meta['object_names']
   if not ob.hide_render:ob['asset_group']=meta['asset_id']
 manifest=out/'views.json';write_json(manifest,meta);scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(manifest,out/'actual',width=384);sheet([out/'actual'/f'view-{i}-textured.png'for i in range(8)],out/'template-eight.png');authority=json.loads((BASE/'mound-ownership-v1/report.json').read_text());row=next(r for r in authority['records']if r['site']=='site-16');names={r['object']for r in row['objects']};own=[scene.objects[n]for n in names]
 for ob in scene.objects:
  if ob.type=='MESH':ob.hide_render=ob.name not in names
 paths=[]
 for label,direction in [('native',RAY),('side',Vector((COS,0,SIN))),('low-side',Vector((math.cos(math.radians(12)),0,math.sin(math.radians(12)))) )]:
  frame(scene,own,direction,480,1.5);p=out/f'site-16-{label}.png';scene.render.filepath=str(p);bpy.ops.render.render(write_still=True);paths.append(p)
 sheet(paths,out/'site-16-three.png');assert sha(model)==proof['model_sha256'];write_json(out/'report.json',dict(model_sha256=sha(model),template_source_cameras_first=True,site16_scope='Isolated material exposure only; source/contact geometry remains exactly unchanged. Ground and wall contact rerenders separate.',images={p.name:sha(p)for p in out.glob('*.png')}))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
