"""Freeze gate-only native texture authority without transferring boundary pixels."""
import hashlib,json
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/gate-geometry-v10';SRC=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/gate-texture-authority-v1'
if OUT.exists():raise FileExistsError(OUT)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
approved=json.loads((BASE/'user-geometry-approval.json').read_text());audit=json.loads((BASE/'native-first-hit-audit.json').read_text());record=next(r for r in json.loads((SRC/'manifest.json').read_text())['records'] if r['id']=='patch-000');OUT.mkdir(parents=True)
rows=[]
for state,row,index in [('covered',0,0),('raised',1,44)]:
 a=next(r for r in audit['states'] if r['state']==state);member=next(r for r in approved['members'] if r['asset_id']=='york-castle-portcullis--'+state);assert sha(Path(member['model']))==member['model_sha256']==a['model_sha256'];f=record['rows'][row]['frames'][index];image=Image.open(SRC/f['image']).convert('RGBA');domain=image.getchannel('A').point(lambda v:255 if v>=128 else 0)
 for x,y,owner in a['not_gate']:domain.putpixel((x-f['bbox'][0],y-f['bbox'][1]),0)
 p=OUT/state;p.mkdir();image.save(p/'native-source.png');domain.save(p/'known-gate-domain.png');pixels=sum(v>0 for v in domain.getdata());assert pixels==a['pixel_center_first_hits']['scenery-york-castle-portcullis']
 rows.append({'state':state,'approved_model_sha256':member['model_sha256'],'source_bbox':f['bbox'],'native_source_sha256':sha(p/'native-source.png'),'known_domain_sha256':sha(p/'known-gate-domain.png'),'known_gate_pixels':pixels,'excluded_boundary_pixels':a['not_gate']})
report={'status':'Gate source authority prepared; UV bake and provider packet pending. Jamb authority separate and unresolved.','approval_receipt_sha256':approved['receipt_sha256'],'audit_sha256':sha(BASE/'native-first-hit-audit.json'),'source_manifest_sha256':sha(SRC/'manifest.json'),'states':rows,'rules':['Only native patch000 opaque pixels whose first hit is the approved movable gate are known gate appearance.','Excluded boundary pixels retain their recorded receivers; no material or geometry edit hides these residuals.','Transparent source holes remain transparent source evidence, never filled by projection.','Unknown hidden gate surfaces require supplemental same-map timber reference; geometry and known pixels must remain exact.','New jamb belongs to building778; no patch000 color assignment to jamb or other gatehouse.']}
(OUT/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'output':str(OUT),'counts':[r['known_gate_pixels'] for r in rows]}))
