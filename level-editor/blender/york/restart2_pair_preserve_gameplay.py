"""Carry unchanged native gameplay metadata into the approved pair's private export."""
import copy,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/pair-textures-v1/assembled-v3-bounded';SRC=BASE/'export-v1';OUT=BASE/'export-v2';LIVE=ROOT/'level-editor/library/3d-assets/york'
if OUT.exists():raise FileExistsError(OUT)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
shutil.copytree(SRC/'3d-assets',OUT/'3d-assets');rows=[]
for asset in ['york-market-southeast-tall-narrow-house','york-southwest-square-west-house']:
 file=OUT/'3d-assets'/asset/'asset.json';new=json.loads(file.read_text());live_file=LIVE/asset/'asset.json';old=json.loads(live_file.read_text())
 assert new['source_origin_scene']==old['source_origin_scene']
 assert {p['node'] for p in new['parts']}=={p['node'] for p in old['parts']}
 for part in new['parts']:
  old_part=next(p for p in old['parts'] if p['node']==part['node'])
  assert part.get('source_obstacle')==old_part.get('source_obstacle')
 new['gameplay']=copy.deepcopy(old['gameplay']);file.write_text(json.dumps(new,indent=2)+'\n')
 model=file.with_name('model.glb');assert sha(model)==sha(SRC/'3d-assets'/asset/'model.glb')
 rows.append({'asset_id':asset,'live_descriptor_sha256':sha(live_file),'descriptor_sha256':sha(file),'approved_export_model_sha256':sha(model),'source_pivot_preserved':True,'source_part_ids_preserved':True,'gameplay_preserved_verbatim':True,'gameplay_sha256':hashlib.sha256(json.dumps(new['gameplay'],sort_keys=True,separators=(',',':')).encode()).hexdigest()})
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
write_asset_index(OUT/'3d-assets')
for name in ['surface-verification.json','export-report.json']:shutil.copy2(SRC/name,OUT/name)
(OUT/'gameplay-preservation.json').write_text(json.dumps({'status':'PASS exact approved models and native gameplay retained','source_export':str(SRC),'members':rows,'requires_browser_recheck':True,'live_integrated':False},indent=2)+'\n')
print(OUT)
