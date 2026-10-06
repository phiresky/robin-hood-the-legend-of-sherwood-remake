"""Freeze a private combined state overlay without changing installed resources."""
import hashlib,json,os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart2-state'
LIVE=ROOT/'level-editor/library'
OUT=BASE/'remaining-seven-package-v2'
def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p): return json.loads(p.read_text())
def save(p,d): p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(d,indent=2)+'\n')
index=read(LIVE/'mission-states/index.json'); baseline=digest(LIVE/'mission-states/index.json')
assert len(index['entries'])==34 and baseline=='9030db7eaf6358f800cc88072d7919799c4911237b72a99a9ef455e58a430789'
files={}; reused={}; entries=[]
def stage(path,source,sha):
 assert digest(source)==sha,(path,'source changed')
 if (LIVE/path).exists():
  assert digest(LIVE/path)==sha,(path,'installed conflict');reused[path]={'path':path,'sha256':sha};return
 dest=OUT/'library'/path;dest.parent.mkdir(parents=True,exist_ok=True)
 if dest.exists(): assert digest(dest)==sha
 else: os.link(source,dest)
 files[path]={'path':path,'sha256':sha,'bytes':dest.stat().st_size}
for name in ['s03-combined-log-package-v1','cart-mission-package-v1']:
 m=read(BASE/name/'manifest.json');assert m['installed_index_sha256']==baseline
 for e in m['entries']:
  e=dict(e);e['kind']='transition';entries.append(e)
 for f in m['files']:
  if f['path']!='mission-states/index.json':stage(f['path'],BASE/name/'library'/f['path'],f['sha256'])
 for f in m['reused']:stage(f['path'],LIVE/f['path'],f['sha256'])
fbase=BASE/'fence-contract-preparation-v2';fm=read(fbase/'manifest.json')
for f in fm['resources']:stage(f['path'],Path(f['source']),f['sha256'])
for r in fm['records']:
 mission=r['mission'];original=fbase/r['contract'];assert digest(original)==r['sha256']
 contract=read(original)
 for binding in contract['families'][0]['physical']['applied']:
  if binding['id']=='croisement02-cleared-fence-source-overlay':binding['position']=[0,0,0]
 source=OUT/'explicit-placement-contracts'/Path(r['contract']).name;save(source,contract)
 path='mission-states/croisement02/'+r['contract'];sha=digest(source);stage(path,source,sha)
 template=next(e for e in index['entries'] if e['mission']==mission)
 entries.append({**template,'kind':'transition','id':mission.lower()+'-south-field-fence','name':'South field fence','contract':{'path':path,'sha256':sha}})
 # Every declared path/hash pair must resolve to the staged or installed immutable resource.
 def walk(v):
  if isinstance(v,dict):
   if 'path' in v and 'sha256' in v:
    p=v['path'];s=OUT/'library'/p
    if not s.exists():stage(p,LIVE/p,v['sha256'])
    else:assert digest(s)==v['sha256']
   for x in v.values():walk(x)
  elif isinstance(v,list):
   for x in v:walk(x)
 walk(read(source));walk(entries[-1])
assert len(entries)==7 and len({e['id'] for e in index['entries']+entries})==41
newindex={**index,'entries':index['entries']+entries}; ip=OUT/'library/mission-states/index.json';save(ip,newindex)
manifest={'status':'PRIVATE_PREPARED; actual matching-scene Editor verification pending','installed_index_sha256':baseline,'preserved_entries':34,'entries':entries,'files':list(files.values()),'reused':list(reused.values()),'private_index':{'path':'mission-states/index.json','sha256':digest(ip),'bytes':ip.stat().st_size},'static_map_sha256':'9b425c58163fb0a61c03b4c9f63cd7863017a3a155300fee0d525ab1e9a60aa5','static_placement_pins_sha256':fm['placement_pins_sha256'],'scope':'Seven independent controlled previews; S03 uses transition catalog kind with unchanged reviewed contract; no actor or automatic mission-script execution. Exact fence replacement requires matching refined scene.'}
save(OUT/'manifest.json',manifest)
assert digest(LIVE/'mission-states/index.json')==baseline
print(json.dumps({'entries':len(entries),'files':len(files),'bytes':sum(f['bytes'] for f in files.values()),'reused':len(reused),'manifest_sha256':digest(OUT/'manifest.json')}))
