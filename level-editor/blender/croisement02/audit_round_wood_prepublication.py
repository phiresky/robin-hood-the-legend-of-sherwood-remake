"""Independently audit frozen staged publication against live bytes without mutations."""
from pathlib import Path
import json,hashlib,mmap,datetime
R=Path.cwd();L=R/'level-editor/library';B=R/'level-editor/work/active/croisement02/approved-round-wood-integration-oct11-v1';S=B/'publication/stage-v3-current';O=B/'prepublication-audit-oct12-v1'
def sha(p):
 if not p.exists():return None
 h=hashlib.sha256()
 with p.open('rb') as f:
  for c in iter(lambda:f.read(1048576),b''):h.update(c)
 return h.hexdigest()
def read(p):return json.loads(p.read_text())
def main():
 O.mkdir(exist_ok=False);errors=[];checks=[]
 def check(name,ok,detail=None):
  checks.append({'name':name,'pass':bool(ok),'detail':detail})
  if not ok:errors.append(name)
 plan=read(S/'plan.json');expected='f5a74976a9e72c378ab216d9a370b2f2edf40cb1c01e11341b64c3c0943c89fb';check('Exact frozen plan',sha(S/'plan.json')==expected)
 check('Exactly12 unique targets',len(plan['records'])==12 and len({r['target']for r in plan['records']})==12)
 for row in plan['records']:
  target=Path(row['target']);source=Path(row['source']);old=sha(target);new=sha(source);stage=Path(plan['overlay_library'])/target.relative_to(L)
  check('Target baseline '+str(target.relative_to(L)),old==row['old_sha256'],{'expected':row['old_sha256'],'actual':old})
  check('Candidate and staged resource '+str(target.relative_to(L)),new==row['new_sha256'] and sha(stage)==new,{'expected':row['new_sha256'],'actual':new,'staged':sha(stage)})
 for pin in [plan['approval'],plan['isolated_browser'],plan['delta'],plan['executor'],*plan['protected_descriptors']]:
  p=Path(pin['path']);p=p if p.is_absolute() else R/p;check('Authority '+p.name,sha(p)==pin['sha256'],str(p))
 live=read(L/'scenes/croisement02.rhlos-map.json');staged=read(S/'croisement02.rhlos-map.json');diff=[]
 for a,c in zip(live['assetSources'],staged['assetSources']):
  if a!=c:diff.append({'id':a['id'],'fields':[k for k in set(a)|set(c)if a.get(k)!=c.get(k)]})
 aa=dict(live);cc=dict(staged);aa.pop('assetSources');cc.pop('assetSources')
 check('Only two map model pins change',aa==cc and len(live['assetSources'])==len(staged['assetSources']) and len(diff)==2 and {d['id'] for d in diff}=={'croisement02-tree-32','croisement02-tree-38'} and all(d['fields']==['model_sha256']for d in diff),diff)
 preserved=[]
 for idx in (19,20,85,98):
  a=live['assetSources'][idx];c=staged['assetSources'][idx];preserved.append({'index':idx,'id':a['id'],'descriptor_sha256':a['descriptor_sha256']});check('Preserved current descriptor '+a['id'],a['descriptor_sha256']==c['descriptor_sha256'])
 # Compare every byte outside the two compact catalog rows without loading the full catalog.
 def row(mm,aid):
  token=(' {"id":'+json.dumps(aid)).strip().encode();start=mm.find(token);assert start>=0 and mm.find(token,start+1)<0
  depth=0;quoted=False;escaped=False
  for end in range(start,len(mm)):
   c=mm[end]
   if quoted:
    if escaped:escaped=False
    elif c==92:escaped=True
    elif c==34:quoted=False
   elif c==34:quoted=True
   elif c==123:depth+=1
   elif c==125:
    depth-=1
    if depth==0:return start,end+1,json.loads(mm[start:end+1])
  raise ValueError('Unterminated row')
 with (L/'3d-assets/index.json').open('rb')as af,(S/'index.json').open('rb')as cf,mmap.mmap(af.fileno(),0,access=mmap.ACCESS_READ)as am,mmap.mmap(cf.fileno(),0,access=mmap.ACCESS_READ)as cm:
  ar=[];cr=[]
  for aid in ('croisement02-tree-32','croisement02-tree-38'):
   a=row(am,aid);c=row(cm,aid);ar.append(a);cr.append(c);original=dict(a[2]);original['preview_model']=f'croisement02/{aid}/preview.glb';check('Catalog row changes preview only '+aid,original==c[2])
  ap=cp=0;unchanged=True
  for a,c in [*zip(sorted(ar),sorted(cr)),((len(am),len(am),{}),(len(cm),len(cm),{}))]:
   if a[0]-ap!=c[0]-cp:unchanged=False
   else:
    for off in range(0,a[0]-ap,1048576):
     size=min(1048576,a[0]-ap-off)
     if am[ap+off:ap+off+size]!=cm[cp+off:cp+off+size]:unchanged=False
   ap=a[1];cp=c[1]
  check('All unrelated catalog bytes identical',unchanged)
 proof=read(Path(plan['isolated_browser']['path']))
 for r in plan['records']:
  if Path(r['target']).name=='model.glb':check('Exact approved browser model '+Path(r['target']).parent.name,proof['approved_models'][Path(r['target']).parent.name]==r['new_sha256'])
 report={'status':'PASS_PREPUBLICATION_BYTES_ONLY' if not errors else 'FAIL_DRIFT_OR_SCOPE','utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'plan_sha256':sha(S/'plan.json'),'checks':checks,'mismatches':errors,'preserved_four_descriptors':preserved,'limitation':'Read-only prepublication audit; running fresh browser evidence and root acceptance still required. This does not authorize apply or replace runtime proof.','canonical_writes':False,'stage_writes':False};(O/'audit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'status':report['status'],'checks':len(checks),'mismatches':errors,'report_sha256':sha(O/'audit.json')}))
if __name__=='__main__':main()
