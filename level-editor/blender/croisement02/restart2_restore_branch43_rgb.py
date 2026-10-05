"""Restore two audited neutral contour texels on the continuous43 private candidate."""
import json,sys,shutil
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry,validate
from audit_candidates import audit
from render_tree import render_workspace

def main():
 old=OUT/'restart2-wood/branch-projected-v3/assets/croisement02-tree-43';worker=OUT/'restart2-wood/branch-rgb-v3/assets'/old.name;report=json.loads((OUT/'restart2-wood/tree43-branch-review-v2/texel-audit.json').read_text())
 if sha(old/'model.blend')!=report['model_sha256']:raise ValueError('Stale source audit')
 if worker.exists():raise FileExistsError(worker)
 shutil.copytree(old,worker);bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'];before={o.name:_geometry(o,protect_appearance=False) for o in objects};rows=[r for r in report['samples'] if r['exact_error']>0]
 if {tuple(r['pixel']) for r in rows}!={(1377, 1018), (1376, 1031)}:raise ValueError('Unexpected sample set')
 grouped={}
 for r in rows:grouped.setdefault(r['atlas_name'],[]).append(r)
 for name,samples in grouped.items():
  im=bpy.data.images[name];data=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(data);old_data=data.copy();w,h=im.size;pixels=data.reshape(h,w,4);changes=[]
  for r in samples:
   x,y=r['atlas_pixel'];current=np.rint(pixels[y,x,:3]*255).astype(int)
   if current.tolist()!=r['atlas_rgb'] or len(set(current))!=1:raise ValueError('Only exact audited neutral texel may change')
   pixels[y,x,:3]=np.array(r['source_rgb'])/255.;changes.append((x,y))
  changed=np.any(data.reshape(h,w,4)!=old_data.reshape(h,w,4),axis=2)
  if int(changed.sum())!=len(set(changes)):raise ValueError('Outside atlas texel changed')
  im.pixels.foreach_set(data);im.update();im.pack()
 if before!={o.name:_geometry(o,protect_appearance=False) for o in objects}:raise ValueError('Geometry/UV changed')
 validate(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
 write_json(worker/'inspection/contour-rgb-restoration.json',dict(model_sha256=sha(worker/'model.blend'),previous_model_sha256=report['model_sha256'],changed_texels=rows,other_texels_unchanged=True,geometry_uv_unchanged=True,ownership='Two root-reviewed inferred native contour samples, exact native RGB. No inferred repaint or semantic color replacement.',approval='pending'))
 audit(worker);render_workspace(worker,384,release_slot=False)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
