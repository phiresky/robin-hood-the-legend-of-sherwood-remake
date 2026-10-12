"""Explicit index-only rebase; immutable browser proof retains its original scope."""
import argparse,copy,hashlib,json,re
from pathlib import Path
import publish_approved_round_wood as original
R,L=original.ROOT,original.LIB
V3=original.RUNS/'stage-v3-current'
V3_SHA='f5a74976a9e72c378ab216d9a370b2f2edf40cb1c01e11341b64c3c0943c89fb'
IDS={'croisement02-tree-32','croisement02-tree-38'}
PAT=re.compile(rb'\{"id":"([^"\\]+)","name":"(?:[^"\\]|\\.)*","source_map":')
sha=original.sha

def require(ok,message):
 if not ok:raise ValueError(message)
def read(p):return json.loads(p.read_text())
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
def rows(data):
 matches=list(PAT.finditer(data));require(data.startswith(b'{"version":1,"assets":[')and data.rstrip().endswith(b']}'),'Unexpected compact catalog format')
 result={}
 for i,m in enumerate(matches):
  end=matches[i+1].start()if i+1<len(matches)else len(data.rstrip())-2
  if i+1<len(matches):require(data[end-1:end]==b',','Row separator changed');end-=1
  aid=m.group(1).decode();require(aid not in result,'Duplicate row');result[aid]=(m.start(),end,data[m.start():end])
 require(len(result)>0,'No catalog rows');return result

def candidate(current):
 rs=rows(current);out=current
 for aid in sorted(IDS,key=lambda a:rs[a][0],reverse=True):
  start,end,raw=rs[aid];r=json.loads(raw);r['preview_model']=f'croisement02/{aid}/preview.glb';out=out[:start]+json.dumps(r,separators=(',',':')).encode()+out[end:]
 return out

def guard_scope(reviewed,current,new):
 a,b,c=rows(reviewed),rows(current),rows(new)
 require(set(a)==set(b)==set(c),'Catalog membership drift')
 protected=set()
 for catalog in (a,b,c):
  for aid,(_,_,raw) in catalog.items():
   parsed=json.loads(raw);source_map=parsed.get('source_map','')
   require(isinstance(source_map,str),'Invalid source_map: '+aid)
   if source_map.casefold()=='croisement02' or aid.casefold().startswith('croisement02-'):protected.add(aid)
 require([i for i in a if i in protected]==[i for i in c if i in protected],'C02 row ordering drift')
 for aid in a:
  if aid in protected:require(a[aid][2]==c[aid][2],'Reviewed C02 row changed: '+aid)
  if aid not in IDS:require(b[aid][2]==c[aid][2],'Unrelated current row not preserved: '+aid)
 require(candidate(current)==new,'Unexpected catalog byte change')
 return [aid for aid in a if a[aid][2]!=b[aid][2]and aid not in IDS]

def guard_records(records,field='old_sha256'):
 for r in records:require(sha(Path(r['target']))==r[field],'Target drift: '+r['target'])

def authoritative_plan():
 require(sha(V3/'plan.json')==V3_SHA,'Original plan drift');p=read(V3/'plan.json')
 for key in ('executor','delta','approval','isolated_browser'):
  pin=p[key];path=Path(pin['path']);path=path if path.is_absolute()else R/path
  require(sha(path)==pin['sha256'],'Original authority drift: '+key)
 return p

def guard_required_checks(plan,authority):
 require(plan['editor_checks_required']==authority['editor_checks_required'],'Required editor checks differ from immutable v3')

def guard_audit_bindings(plan,audit):
 expected={'path':str(V3/'plan.json'),'sha256':V3_SHA}
 require(plan['original_plan']==expected and audit['original_plan']==expected,'Audit original plan binding differs')
 require(audit['current_index_sha256']==plan['records'][-1]['old_sha256'],'Audit old index binding differs')
 require(audit['new_index_sha256']==plan['records'][-1]['new_sha256'],'Audit new index binding differs')
 require(audit['unchanged_map_and_delivery_records']==11,'Audit delivery scope differs')

def prepare(out):
 require(not out.exists(),'New output required');p=authoritative_plan();require(len(p['records'])==12,'Expected12records')
 guard_records(p['records'][:-1]);original.validate_inputs()
 for r in p['records']:require(sha(Path(r['source']))==r['new_sha256'],'Original candidate drift')
 for pin in p['protected_descriptors']:original.io.core.verify_pin(pin)
 live=L/'3d-assets/index.json';before=sha(live);current=live.read_bytes();reviewed=Path(p['records'][-1]['source']).read_bytes();new=candidate(current);changed=guard_scope(reviewed,current,new);require(sha(live)==before,'Index changed during prepare')
 out.mkdir();ip=out/'index.json';ip.write_bytes(new);records=copy.deepcopy(p['records']);records[-1].update(source=str(ip),old_sha256=before,new_sha256=sha(ip))
 audit={'status':'PASS_UNCHANGED_C02_SCOPE_CURRENT_UNRELATED_ROWS_PRESERVED','original_plan':{'path':str(V3/'plan.json'),'sha256':V3_SHA},'current_index_sha256':before,'new_index_sha256':sha(ip),'preserved_non_c02_updates':changed,'unchanged_map_and_delivery_records':11,'limitations':'No new browser proof and no relabeling. Original v3 six-behavior proof must pass and be explicitly bound separately before apply.'};dump(out/'audit.json',audit)
 plan={'status':'DRAFT_REBASED_ORIGINAL_BROWSER_PROOF_PENDING','records':records,'original_plan':audit['original_plan'],'audit':{'path':str(out/'audit.json'),'sha256':sha(out/'audit.json')},'protected_descriptors':p['protected_descriptors'],'executor':{'path':str(Path(__file__).resolve()),'sha256':sha(Path(__file__).resolve())},'editor_checks_required':p['editor_checks_required']};dump(out/'plan.json',plan)
 return plan

def bind(out,proof):
 plan=read(out/'plan.json');authority=authoritative_plan();guard_required_checks(plan,authority);original.io.core.verify_pin(plan['audit']);guard_audit_bindings(plan,read(Path(plan['audit']['path'])));e=read(proof);require(e.get('status','').startswith('PASS'),'Original browser proof must pass');require(e.get('plan_sha256')==V3_SHA,'Browser proof must retain original v3 binding');require(all(e.get('editor_checks',{}).get(k)is True for k in plan['editor_checks_required']),'Six editor behaviors required')
 result={'status':'PASS_COMPOSITE_UNCHANGED_C02_SCOPE_ONLY','plan_sha256':sha(out/'plan.json'),'original_browser_proof':{'path':str(proof),'sha256':sha(proof)},'original_plan_sha256':V3_SHA,'rebase_audit':plan['audit'],'editor_checks':e['editor_checks'],'scope':'Original v3 browser behavior applies only to byte-identical C02 rows/map/deliveries. Non-C02 metadata preserved but not newly browser-tested. Short actual canonical smoke remains required.'};dump(out/'composite.json',result)

def verify(out):
 plan=read(out/'plan.json');authority=authoritative_plan();guard_required_checks(plan,authority);composite=read(out/'composite.json');require(composite['plan_sha256']==sha(out/'plan.json'),'Composite plan drift');require(sha(V3/'plan.json')==V3_SHA,'Original plan drift');original.io.core.verify_pin(plan['executor']);original.io.core.verify_pin(plan['audit']);original.io.core.verify_pin(composite['original_browser_proof'])
 guard_audit_bindings(plan,read(Path(plan['audit']['path'])));require(composite['rebase_audit']==plan['audit'] and composite['original_plan_sha256']==V3_SHA,'Composite audit/original binding differs')
 proof=read(Path(composite['original_browser_proof']['path']));require(proof.get('status','').startswith('PASS')and proof.get('plan_sha256')==V3_SHA,'Original proof scope changed');require(all(proof.get('editor_checks',{}).get(k)is True for k in plan['editor_checks_required']),'Missing editor behavior')
 for path,digest in proof.get('evidence',{}).items():require(sha(R/path)==digest,'Original proof evidence drift: '+path)
 original.validate_inputs();guard_records(plan['records']);require(plan['records'][:-1]==authority['records'][:-1],'Map/delivery records altered')
 require(plan['records'][-1]['target']==str(L/'3d-assets/index.json'),'Index target altered')
 for r in plan['records']:require(sha(Path(r['source']))==r['new_sha256'],'Candidate drift')
 for pin in plan['protected_descriptors']:original.io.core.verify_pin(pin)
 guard_scope((V3/'index.json').read_bytes(),(L/'3d-assets/index.json').read_bytes(),(out/'index.json').read_bytes())
 return plan

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,required=True);ap.add_argument('--proof',type=Path);ap.add_argument('--apply',action='store_true');ap.add_argument('--root-reviewed',action='store_true');ap.add_argument('--readers-quiescent',action='store_true');args=ap.parse_args();out=args.output.resolve();require(out.parent==original.RUNS,'Wrong private output location')
 with original.io.core.locked():
  if args.apply:
   require(args.root_reviewed and args.readers_quiescent,'Root review and quiescent readers required');plan=verify(out)
   original.io.commit(plan['records'],out/'transaction',sha(out/'plan.json'),lambda:verify(out),lambda:original.io.verify(plan['records'],'new_sha256'))
  elif args.proof:bind(out,args.proof.resolve())
  else:prepare(out)
 print(out)
if __name__=='__main__':main()
