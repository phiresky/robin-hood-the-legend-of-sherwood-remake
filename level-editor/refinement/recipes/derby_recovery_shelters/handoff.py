"""Emit scoped import fragments only after validating all saved bake evidence."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
root=Path.cwd()/'level-editor/work/needs-review/derby-refinement/round-2/recovery-shelters-20260923'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
imports=[];reports={}
for asset in ['derby-east-wall-landing','derby-lower-northwest-cottage-barrel','derby-east-courtyard-south-shelter']:
 w=root/asset
 bake=w/('baked-chimney-retry' if (w/'baked-chimney-retry/actual/textured.png').exists() else 'baked')
 exp=w/('experiment-chimney-retry' if bake.name=='baked-chimney-retry' else 'experiment')
 if not (bake/'actual/textured.png').exists():continue
 v=json.loads((bake/'validation.json').read_text());m=json.loads((exp/'views.json').read_text())
 assert v['geometry_verified'] and v['outside_objects_unchanged']>700
 for p,h in v['evidence_sha256'].items():assert sha(Path(p))==h
 generated=Path(v['generated_image']); original=np.asarray(Image.open(exp/'input.png').convert('RGBA'));result=np.asarray(Image.open(generated).convert('RGBA'));protected=np.asarray(Image.open(exp/'mask.png').convert('RGBA'))[:,:,3]>0
 changed=int(np.any(original[protected]!=result[protected],axis=1).sum());assert changed==0
 nodes={'derby-east-wall-landing':['building-069'],'derby-lower-northwest-cottage-barrel':['building-062'],'derby-east-courtyard-south-shelter':['building-067','building-068']}[asset]
 evidence={str(Path(p).resolve()):h for p,h in v['evidence_sha256'].items()}
 assert evidence[str((exp/'views.json').resolve())]==sha(exp/'views.json')
 imports.append({'asset_id':asset,'blend_path':str(bake/'worker.blend'),'blend_sha256':sha(bake/'worker.blend'),'object_names':m['texture_receiver_object_names'],'source_nodes':nodes,'review_manifest':str(exp/'views.json'),'approval_evidence':str(exp/'approval.json'),'geometry_baseline':str(w/'model.blend'),'validation':str(bake/'validation.json')})
 reports[asset]={'protected_pixels_changed':changed,'protected_pixels':int(protected.sum()),'outside_objects_unchanged':v['outside_objects_unchanged'],'geometry_verified':True,'actual_eight_views':str(bake/'actual/textured.png'),'actual_sha256':sha(bake/'actual/textured.png'),'counts':v['counts'],'raw_retained':str(generated.parent/'generated-raw.png'),'source_mask_evidence':v['source_mask_evidence']}
(root/'imports.json').write_text(json.dumps(imports,indent=2))
(root/'handoff.json').write_text(json.dumps(reports,indent=2))
print(json.dumps({k:{x:v[x] for x in ['protected_pixels_changed','outside_objects_unchanged','geometry_verified']} for k,v in reports.items()}))
