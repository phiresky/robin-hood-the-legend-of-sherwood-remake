"""Inject exact native phases into saved review geometry; no runtime or model writes."""
import sys,json,hashlib
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_full_crown_saved as full
B=full.B;I=B/'tree01-native-transition-cpu-v2';FOLDER=B/'tree01-native-transition-saved-v2';O=FOLDER/'phase-geometry-proof-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not O.exists();full.review.resources();full.review.acquire(slots=2)
 try:
  full.review.resources();O.mkdir();model=FOLDER/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;obj=scene.objects['Tree01 observed native frame0 provisional cells'];tex=next(n for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');original=tex.image;data=json.loads((I/'cells-native.json').read_text());rows=[];full.OBJECT_NAME=obj.name
  for phase in range(14):
   source=B/'tree01-native-fragment-v1'/f'{phase:03}.png';packet=dict(data);packet['source_frame']=str(source);packet['source_frame_sha256']=sha(source);packet['source_role']=f'diagnostic native phase{phase} injection';path=O/f'phase-{phase:02}-cells.json';path.write_text(json.dumps(packet,separators=(',',':'))+'\n');image=bpy.data.images.load(str(source),check_existing=False);image.pack();tex.image=image;full.CELLS=path;full.GUARD_NAME=f'phase-{phase:02}-firsthits.json';full.verify(scene,O);receipt=json.loads((O/full.GUARD_NAME).read_text());rows.append({'phase':phase,'source_sha256':sha(source),'pixels':receipt['pixels'],'failures':len(receipt['failures'])});tex.image=original;bpy.data.images.remove(image)
  assert sha(model)==digest;report={'status':'PASS_NATIVE_PHASE_GEOMETRY_INJECTION','model_sha256':digest,'phases':rows,'limits':['Temporary exact phase textures applied in memory to saved review geometry; saved model remains frame0 and byte-identical.','This proves source first-hit compatibility, not native clock or editor integration.','Full shared observed animation still requires one owner; do not publish diagnostic native cells beside it.']};(O/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print({'phases':len(rows),'source_pixels_tested':sum(r['pixels']for r in rows),'status':report['status']})
 finally:full.review.release()
if __name__=='__main__':main()
