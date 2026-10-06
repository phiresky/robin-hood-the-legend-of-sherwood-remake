"""Prepare a small contact-view fixture using unchanged verified delivery resources."""
import hashlib,json,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement02-refinement';S=B/'restart2-state/remaining-seven-package-v2';OUT=B/'restart2-state/remaining-seven-contact-v1';STATIC=B/'restart2-textures/batch10-private-browser-delivery-v1/map-assets'
def read(p):return json.loads(p.read_text())
def pin(p):return {'url':'/@fs'+str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
m=read(S/'manifest.json');doc=read(STATIC/'scenes/croisement02.rhlos-map.json');records=[]
for id in ['s03_fob_mp-log-trap','emb05_fob_mp-south-cart','emb09_fob_jms-north-cart','emb05_fob_mp-south-field-fence']:
 e=next(e for e in m['entries'] if e['id']==id);cp=S/'library'/e['contract']['path'];c=read(cp);f=c['families'][0];record={'id':id,'contract':pin(cp),'family':f,'models':{}}
 for endpoint in ['initial','applied']:
  for binding in f['physical'][endpoint] if isinstance(f['physical'][endpoint],list) else []:
   p=S/'library'/binding['model']
   if not p.exists():p=ROOT/'level-editor/library'/binding['model']
   r=pin(p);assert r['sha256']==binding['model_sha256'];record['models'][binding['model']]=r
 records.append(record)
receivers=[]
for id in ['croisement02-terrain','croisement02-north-woodland-bank','croisement02-northeast-oak-root-bank']:
 src=next(x for x in doc['assetSources']+doc['sceneAssets'] if x['id']==id);model=pin(STATIC/src['model']);assert model['sha256']==src['model_sha256']
 if id.endswith('terrain'):position=[0,0,0];rotation=[-math.pi/2,0,0]
 else:
  placement=next(p for p in doc['placements'] if p['id']==id);t=placement['transform'];assert t['rot_deg']==0 and t['dz']==0
  position=[t['dx'],0,t['dy']/math.sin(math.radians(35))];rotation=[0,0,0]
 receivers.append({'id':id,'model':model,'position':position,'rotation':rotation,'descriptor':pin(STATIC/src['descriptor'])})
OUT.mkdir(parents=True,exist_ok=True)
(OUT/'manifest.json').write_text(json.dumps({'scope':'Focused contact diagnostic only. Exact same state endpoints and ground/bank resources as private refined scene; foliage, actors and unrelated static scenery excluded explicitly. Native camera direction first, opposite direction supplementary. No full-scene occlusion parity claim.','package':pin(S/'manifest.json'),'static_document':pin(STATIC/'scenes/croisement02.rhlos-map.json'),'records':records,'receivers':receivers},indent=2)+'\n')
(OUT/'index.html').write_text('<!doctype html><meta charset="utf-8"><style>body{margin:0;background:#303030;color:white;font:15px sans-serif}#title{padding:8px}canvas{display:block}</style><div id="title">Focused state contact diagnostic</div><script type="module" src="./proof.ts"></script>')
(OUT/'proof.ts').write_bytes((ROOT/'level-editor/blender/croisement02/restart6_state_contact_browser.ts').read_bytes())
print(OUT)
