"""Reopen terrain-bound leaf endpoints with their exact support surfaces."""
from pathlib import Path
import sys,json,hashlib,math
import bpy
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from restart4_stump_final_contact import frame,sheet
from sign_context_import import append_verified
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=OUT/'restart9-hiding-scatter';worker=root/'scatter-surfaces-v2';out=root/'scatter-terrain-context-v2';out.mkdir(exist_ok=False);manifest=json.loads((worker/'manifest.json').read_text());audit=json.loads((root/'terrain-receivers-v2/report.json').read_text());model=worker/'model.blend';assert sha(model)==manifest['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();endpoints=[o for o in scene.objects if o.type=='MESH'];receipts=[]
 for pin in audit['models']:
  path=Path(pin['path']);assert sha(path)==pin['sha256'];names=[r['name']for r in pin['objects']];objects,proof=append_verified(scene,path,names,{r['name']:r for r in pin['objects']});receipts.append(dict(path=str(path),sha256=sha(path),objects=proof))
 scene.cycles.transparent_max_bounces=512;scene.cycles.samples=16;images=[]
 for review in manifest['reviews']:
  record=next(r for r in manifest['records']if r['index']==review['index']);own=[bpy.data.objects[n]for n in record['objects']]
  for o in endpoints:o.hide_render=o not in own
  for name,direction in [('native',RAY),('oblique',Vector((COS,0,SIN)))]:
   frame(scene,own,direction,640,1.7);path=out/f'endpoint-{record["index"]:02}-{name}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);images.append(dict(instance=review['id'],view=name,path=path.name,sha256=sha(path)))
 sheet([out/r['path']for r in images],out/'eight-contexts.png');(out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),receivers=receipts,images=images,scope='Exact terrain contact only; foreground scene ownership and visibility remain separate.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
