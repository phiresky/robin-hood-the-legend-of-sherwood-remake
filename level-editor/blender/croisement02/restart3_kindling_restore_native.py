"""Restore independently audited neutral native samples on unchanged kindling geometry."""
import json,sys,shutil
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json,record_recipe
from render_slots import acquire,release
from refinement_workspace import _geometry,validate,_render
from render_tree import render_workspace

def main():
 old=OUT/'restart3-kindling/candidate-v2/assets/croisement02-southwest-kindling-bundle';worker=OUT/'restart3-kindling/native-rgb-v1/assets'/old.name;audit=OUT/'restart3-kindling/texel-audit-v2/texel-audit.json';report=json.loads(audit.read_text())
 if sha(old/'model.blend')!=report['model_sha256']:raise ValueError('Stale native audit')
 if worker.exists():raise FileExistsError(worker)
 shutil.copytree(old,worker);(worker/'inspection').rename(worker/'previous-inspection');(worker/'inspection').mkdir();(worker/'recipe').rename(worker/'previous-recipe');(worker/'modified').rename(worker/'previous-modified');bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'];before={o.name:_geometry(o,False) for o in objects};outside={o.name:_geometry(o,True) for o in objects if o.get('asset_group')!=old.name};rows=[r for r in report['samples'] if r['neighbor_error']>0 and len(set(r['atlas_rgb']))==1]
 if len(rows)!=58:raise ValueError('Unexpected neutral sample set')
 grouped={}
 for r in rows:grouped.setdefault(r['atlas_name'],[]).append(r)
 for name,samples in grouped.items():
  im=bpy.data.images[name];data=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(data);old_data=data.copy();w,h=im.size;pixels=data.reshape(h,w,4);changes=[]
  for r in samples:
   x,y=r['atlas_pixel'];current=np.rint(pixels[y,x,:3]*255).astype(int)
   if current.tolist()!=r['atlas_rgb']:raise ValueError('Neutral audit changed')
   pixels[y,x,:3]=np.array(r['source_rgb'])/255.;changes.append((x,y))
  if int(np.any(data.reshape(h,w,4)!=old_data.reshape(h,w,4),axis=2).sum())!=len(set(changes)):raise ValueError('Other atlas texel changed')
  im.pixels.foreach_set(data);im.update();im.pack()
 if before!={o.name:_geometry(o,False) for o in objects} or outside!={o.name:_geometry(o,True) for o in objects if o.get('asset_group')!=old.name}:raise ValueError('Geometry/UV or outside appearance changed')
 validate(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));cfg=json.loads((worker/'workspace.json').read_text());_render(cfg,worker/'modified',worker/'input/views.json');proof=json.loads((old/'inspection/refinement.json').read_text());proof['model_sha256']=sha(worker/'model.blend');proof['native_rgb_restoration']=dict(parent_model_sha256=report['model_sha256'],audit_sha256=sha(audit),count=58,only_neutral_changed=True);write_json(worker/'inspection/refinement.json',proof)
 write_json(worker/'inspection/native-rgb-restoration.json',dict(model_sha256=sha(worker/'model.blend'),previous_model_sha256=report['model_sha256'],changed_texels=rows,other_texels_unchanged=True,geometry_uv_unchanged=True,outside_appearance_unchanged=True,ownership='Exact original mask104 first-hit source centers; no generated pixels or colored texel replacements.'));record_recipe(worker,Path(__file__));render_workspace(worker,384,release_slot=False)
 if sha(old/'model.blend')!=report['model_sha256']:raise ValueError('Parent changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
