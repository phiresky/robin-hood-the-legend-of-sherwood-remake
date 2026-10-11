"""Stage the approved Day gate delivery in the normal editor catalog; no live writes."""
import hashlib,json,os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement/restart2'
SOURCE=BASE/'restart48-production-gate-delivery-v2/library'
OUT=BASE/'restart57-gate-catalog-stage-v1'
LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def dump(p,d):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(d,indent=2)+'\n')
def pin(p):return {'path':str(p),'sha256':sha(p)}
def require(v,m):
 if not v:raise RuntimeError(m)
def main():
 require(not OUT.exists(),'Fresh stage required');stage=OUT/'library';stage.mkdir(parents=True)
 index=read(LIB/'mission-states/index.json');entry=read(SOURCE/'mission-states/index.json')['entries'][0]
 require(not any(r['id']==entry['id'] for r in index['entries']),'Gate already registered; reconcile before replacing')
 contract=read(SOURCE/entry['contract']['path']);require(sha(SOURCE/entry['contract']['path'])=='1cb1afa63b3e52d64d8480fc6e9305db197b53f0f1b410e49b73e45e1448575d','Tested contract changed')
 rows=[]
 def copy(source,rel):
  target=stage/rel;target.parent.mkdir(parents=True,exist_ok=True)
  if not target.exists():os.link(source,target)
 for path in (SOURCE/'mission-states/york').rglob('*'):
  if path.is_file() and path.name!='gate.json':copy(path,path.relative_to(SOURCE))
 for state in ['initial','applied']:
  binding=contract['families'][0]['physical'][state][0]
  relative='mission-states/york/models/portcullis-'+state+'-'+binding['model_sha256'][:16]+'.glb'
  copy(SOURCE/binding['model'],relative);binding['model']=relative
 rel='mission-states/york/contracts/portcullis-day-v1.json';dump(stage/rel,contract)
 entry['contract']={'path':rel,'sha256':sha(stage/rel)}
 index['entries'].append(entry);dump(stage/'mission-states/index.json',index)
 for path in sorted(stage.rglob('*')):
  if not path.is_file():continue
  rel=path.relative_to(stage);target=LIB/rel;rows.append({'source':str(path),'target':str(target),'relative':str(rel),'sha256':sha(path),'old_sha256':sha(target) if target.exists() else None,'bytes':path.stat().st_size})
 dump(OUT/'plan.json',{'status':'PRIVATE_GATE_CATALOG_PENDING_NORMAL_EDITOR_PROOF','library':str(stage),'baseline_catalog':pin(LIB/'mission-states/index.json'),'entry':entry,'records':rows,'contract':pin(stage/entry['contract']['path']),'scope':'One Day York mission preview catalog entry; approved endpoint bytes and poses preserved. Final canonical transaction must rebase only this entry under publication lock and switch catalog last. No global asset index or map changes.','source_contract':pin(SOURCE/'mission-states/york/gate.json'),'model_path_change_only':True})
 print(json.dumps({'output':str(OUT),'records':len(rows),'contract':entry['contract']}))
if __name__=='__main__':main()
