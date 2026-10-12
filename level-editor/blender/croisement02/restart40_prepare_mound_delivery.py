"""Bind approved mound appearances to exact source transitions in private contracts."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement02';DEST=BASE/'restart40-approved-mound-export-v2';SOURCE=BASE/'restart7-source-patch-delivery/contracts-v1';LIB=ROOT/'level-editor/library';PLAN=BASE/'restart25-approved-state-materialization-v1/approved-bindings.json';APPROVAL=BASE/'restart3-review-batches/next-tree02-channels-mounds-v1/user-approval.json'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p):return json.loads(Path(p).read_text())
def pin(p):return dict(path=str(p),sha256=sha(p))
def write(p,d):assert not p.exists();p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 plan=read(PLAN);rows=[r for r in plan['bindings']if r['profile']=='Croisement01 - hiding Pc'];assert len(rows)==32;exports={f'site-{i:02d}':read(DEST/f'site-{i:02d}/export.json')for i in range(20)}
 for e in exports.values():assert sha(e['model'])==e['model_sha256'] and sha(e['source_model'])==e['source_sha256'];assert e['approval_sha256']==sha(APPROVAL)
 resources={};fallback={r['path']:r['source']for r in read(SOURCE/'manifest.json')['resources']};mapping={};bindings=[]
 def visit(v):
  if isinstance(v,dict):
   if 'path'in v and 'sha256'in v:
    rel=v['path'];p=LIB/rel
    if not p.is_file()or sha(p)!=v['sha256']:p=Path(fallback[rel])
    assert sha(p)==v['sha256'];item=dict(logical_path=rel,file=str(p),sha256=v['sha256']);assert rel not in resources or resources[rel]==item;resources[rel]=item
   for x in v.values():visit(x)
  elif isinstance(v,list):
   for x in v:visit(x)
 for row in rows:
  source=ROOT/row['source_contract']['path'];assert sha(source)==row['source_contract']['sha256'];native=read(source)['native'];visit(native);patch=next(p for p in native['patch_states']if p['id']==row['id']);duration=sum(f['delay']+1 for f in patch['transition']);assert duration==row['transition_duration'];terminal=max(1,duration-1);assert terminal==row['terminal_tick']
  initial=row['endpoints']['initial'];site=initial['selection']['site'];e=exports[site];assert initial['placement']['kind']=='saved-world-transforms' and initial['placement']['translation']==[0,0,0]
  models={'initial':dict(model=site+'/model.glb',model_sha256=e['model_sha256'],model_scene=e['model_scene'])};mapping[site+'/model.glb']=e['model']
  endpoint=row['endpoints']['applied']
  if endpoint['kind']=='source-absent':
   assert row['id']=='mission-Tac19_FoB_EC-patch-015' and patch['final']==[] and patch['integrate_in_background'] is False
  else:
   applied=endpoint['private_runtime_candidate'];assert sha(applied['path'])==applied['sha256'];assert applied['placement']=='saved-world-transform-no-additional-translation';models['applied']=dict(model='scatter.glb',model_sha256=applied['sha256'],model_scene=applied['model_scene']);mapping['scatter.glb']=applied['path']
  physical={s:([dict(id=row['id']+'-'+s,role='objects',resources=[],position=[0,0,0],**models[s])]if s in models else{'kind':'absent'})for s in ('initial','applied')}
  family=dict(id=row['id']+'-approved-mound',element_ids=[],background_ids=[],patch_ids=[row['id']],body_terminal_tick=terminal,physical=physical);contract=dict(version=1,scope='controlled-state-preview',native=native,families=[family]);path=DEST/(row['id']+'-delivery.json');write(path,contract)
  bindings.append(dict(id=row['id'],mission=row['mission'],site=site,terminal_tick=terminal,transition_duration=duration,contract=pin(path),source_contract=pin(source),physical=physical))
 write(DEST/'reader-config.json',dict(status='PRIVATE_APPROVED_MOUNDS_BROWSER_PENDING',approval=pin(APPROVAL),parent_bindings=pin(PLAN),bindings=bindings,exports={s:pin(DEST/s/'export.json')for s in exports},native_reader=dict(mapping=list(resources.values()),hash_checked=True,no_resource_rewrites=True),asset_handle=dict(root_directory=str(DEST),mapping=mapping,read_only=True),canonical_writes=False,publication_allowed=False,recipe=pin(Path(__file__).resolve()),scope='All20 initial mound appearances across32 source controls. Original transitions,31 applied scatter bindings and absolute placements retained; Tac19patch015 has no applied surface as in its source contract. Browser appearance, lifecycle and receiver context pending. Orphan patch000 absent-initial control remains separate and unmodified.'))
 print(json.dumps(dict(controls=len(bindings),sites=len(exports),native_resources=len(resources))))
if __name__=='__main__':main()
