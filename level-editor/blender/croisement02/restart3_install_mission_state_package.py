from pathlib import Path
import json,hashlib,fcntl,os
r=Path('level-editor/work/croisement02-refinement');p=r/'restart2-state/mission-state-package-v1';lib=Path('level-editor/library');m=json.loads((p/'manifest.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest();proof=r/'restart2-state/full-editor-state-preview-v4/verification.json';assert json.loads(proof.read_text())['status']=='PASS'
with (p/'install.lock').open('a')as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 for x in m['resources']:
  assert x['path'].startswith('mission-states/')and'..'not in Path(x['path']).parts
  b=(p/'library'/x['path']).read_bytes();assert sha(b)==x['sha256']
  dest=lib/x['path']
  if dest.exists():assert dest.read_bytes()==b,('Conflict',str(dest))
 for x in sorted(m['resources'],key=lambda x:x['path']=='mission-states/index.json'):
  dest=lib/x['path'];dest.parent.mkdir(parents=True,exist_ok=True)
  if not dest.exists():
   with dest.open('xb')as f:f.write((p/'library'/x['path']).read_bytes());f.flush();os.fsync(f.fileno())
 for x in m['resources']:assert sha((lib/x['path']).read_bytes())==x['sha256']
 receipt={'status':'INSTALLED','scope':'New mission-states namespace only; scenes and3d-assets catalogs untouched','files':len(m['resources']),'bytes':m['total_bytes'],'index_sha256':sha((lib/'mission-states/index.json').read_bytes()),'manifest_sha256':sha((p/'manifest.json').read_bytes()),'private_editor_verification_sha256':sha(proof.read_bytes()),'next':'Bounded installed-resource full editor verification; no route interception'}
 (p/'installation.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
