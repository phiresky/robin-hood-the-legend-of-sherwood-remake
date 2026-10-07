"""Bounded native-cell depth clearance; keep every source UV/RGBA and wood surface."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main():
 src=B/'tree02-isolated-prototype-v6';out=B/'tree02-isolated-prototype-v7';assert not out.exists();assert shutil.disk_usage(ROOT).free>10*1024**3+128*1024**2;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));s=bpy.data.scenes['Tree02 isolated'];o=next(o for o in s.objects if 'fragment' in o.get('asset_group',''));m=o.data;guard=json.loads((src/'firsthit.json').read_text());receipt=json.loads((src/'construction.json').read_text());assert len(guard['own_leaf_changes'])==7;assert not guard['bark_changes'] and not guard['neighbor_firsthit_changes'];native=receipt['native_faces'];uv=m.uv_layers.active;changes=[];selected=set()
  for x,y,*_ in guard['own_leaf_changes']:
   for f in list(m.polygons)[:native]:
    us=[uv.data[i].uv for i in f.loop_indices];lo=(min(p.x for p in us)*50+175,(1-max(p.y for p in us))*175);hi=(max(p.x for p in us)*50+175,(1-min(p.y for p in us))*175)
    if lo[0]-.001<=x+.5<hi[0]+.001 and lo[1]-.001<=y+.5<hi[1]+.001:selected.add(f.index)
  assert selected;ray=Vector((0,-.8191520442889918,.573576436351046))
  for idx in sorted(selected):
   f=m.polygons[idx]
   for v in f.vertices:m.vertices[v].co+=ray*45
   changes.append(dict(native_cell=idx,source_ray_clearance=45,uv_rgba_projected_footprint_unchanged=True))
  m.update();out.mkdir();bpy.data.libraries.write(str(out/'worker.blend'),{s},fake_user=True,compress=True)
  for name in ('bark.png','native-leaves.png'):shutil.copyfile(src/name,out/name)
  receipt.update(model_sha256=sha(out/'worker.blend'),prior_model_sha256=sha(src/'worker.blend'),native_cell_clearance=changes);write_json(out/'construction.json',receipt);print('SAVED',sha(out/'worker.blend'),'cells',len(changes))
 finally:release()
if __name__=='__main__':main()
