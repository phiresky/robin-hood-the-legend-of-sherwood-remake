"""Bind the revealed counterweight underside to this endpoint's own native pixels."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from log_trap_state_candidate import material
BASE=OUT/'restart2-state/net-empty01-v10';DEST=OUT/'restart2-state/net-empty01-v11'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 acquire()
 try:
  prior=json.loads((BASE/'report.json').read_text());assert sha(BASE/'model.blend')==prior['model_sha256'];DEST.mkdir();bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));source=OUT/'source-states/mission-patches/mission-Emb05_FoB_MP-patch-000/final-000.png';raw=np.array(Image.open(source).convert('RGBA'));known=np.zeros(raw.shape[:2],bool);known[25:51,:24]=True
  for path in BASE.glob('*observed.png'):shutil.copyfile(path,DEST/path.name)
  path=DEST/'Wooden-piece-observed.png';old=np.array(Image.open(path).convert('RGBA'));known|=old[:,:,3]>0;raw[:,:,3]=np.where(known,raw[:,:,3],0);Image.fromarray(raw).save(path);bpy.data.objects['Wooden piece'].data.materials[0]=material(path);bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'));write_json(DEST/'report.json',{**prior,'status':'Private exact empty-phase timber source binding; geometry unchanged fromv10','model_sha256':sha(DEST/'model.blend'),'parent_model_sha256':prior['model_sha256'],'wood_native_source_added_region':[0,25,24,51],'renders':[]})
 finally:release()
if __name__=='__main__':main()
