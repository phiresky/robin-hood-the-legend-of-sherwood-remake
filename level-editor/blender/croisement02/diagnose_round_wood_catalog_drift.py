"""Stream catalog rows to locate independent publication drift with bounded memory."""
import json,hashlib,itertools
from pathlib import Path
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1';OUT=B/'prepublication-audit-oct12-v1/catalog-drift.json'
def rows(p):
 dec=json.JSONDecoder()
 with p.open()as f:
  buf=f.read(1048576);prefix='{"version":1,"assets":[';assert buf.startswith(prefix);buf=buf[len(prefix):]
  while True:
   buf=buf.lstrip(',\n\r ')
   if buf.startswith(']'):return
   try:obj,end=dec.raw_decode(buf)
   except json.JSONDecodeError:
    chunk=f.read(1048576)
    if not chunk:raise
    buf+=chunk;continue
   raw=buf[:end];buf=buf[end:];yield obj,hashlib.sha256(raw.encode()).hexdigest()
def diff(a,b,path=''):
 if a==b:return []
 if isinstance(a,dict)and isinstance(b,dict):
  result=[]
  for k in set(a)|set(b):
   if k not in a or k not in b:result.append({'path':path+'/'+k,'before':a.get(k),'after':b.get(k)})
   else:result+=diff(a[k],b[k],path+'/'+k)
  return result
 if isinstance(a,list)and isinstance(b,list)and len(a)==len(b)and len(a)<100:return sum((diff(x,y,path+'/'+str(i))for i,(x,y)in enumerate(zip(a,b))),[])
 if isinstance(a,(dict,list))or isinstance(b,(dict,list)):return [{'path':path,'before_sha256':hashlib.sha256(json.dumps(a,sort_keys=True).encode()).hexdigest(),'after_sha256':hashlib.sha256(json.dumps(b,sort_keys=True).encode()).hexdigest(),'before_length':len(a),'after_length':len(b)}]
 return [{'path':path,'before':a,'after':b}]
changes=[];count=0
for aa,bb in itertools.zip_longest(rows(B/'publication/stage-v3-current/index.json'),rows(R/'level-editor/library/3d-assets/index.json')):
 count+=1
 if aa is None or bb is None:changes.append({'index':count-1,'rowcount_drift':True});continue
 a,ah=aa;b,bh=bb
 if ah!=bh:changes.append({'index':count-1,'staged_id':a['id'],'canonical_id':b['id'],'staged_row_sha256':ah,'canonical_row_sha256':bh,'fields':diff(a,b)})
report={'status':'READ_ONLY_CATALOG_DRIFT_DIAGNOSIS','row_pairs':count,'changed_rows':changes,'direction':'staged v3 → current canonical. Tree32/38 preview deletions here are expected pending publication; other changes are baseline drift.','canonical_writes':False,'stage_writes':False};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
