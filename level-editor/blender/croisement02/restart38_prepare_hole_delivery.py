"""Prepare private approved hole endpoints without altering source timing or apertures."""
import json,hashlib,copy
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';DEST=BASE/'restart38-approved-hole-export-v1';LIB=ROOT/'level-editor/library';SOURCE=BASE/'restart7-source-patch-delivery/contracts-v1';PLAN=BASE/'restart25-approved-state-materialization-v1/approved-bindings.json'
RECEIPT=BASE/'restart3-review-batches/next-ground-storehouse-hole-v1/user-approval-partial.json';RECEIPT_SHA='db55badd3a8f9545ecbf381f6dc6014d344f8a773e573e055e17bfb7eec5a675'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return dict(path=str(p),sha256=sha(p))
def read(p):return json.loads(Path(p).read_text())
def write(p,d):assert not p.exists();p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert sha(RECEIPT)==RECEIPT_SHA;plan=read(PLAN);assert len(plan['bindings'])==63
 records=[r for r in read(SOURCE/'manifest.json')['records']if r['profile']=='Croisement01 - hole'];assert len(records)==30
 fallback={r['path']:r['source']for r in read(SOURCE/'manifest.json')['resources']};exports={s:read(DEST/s/'export.json')for s in ('initial','applied')};resources={};rows=[];amended=copy.deepcopy(plan);hole_ids={r['id']for r in records}
 for state,r in exports.items():assert sha(r['model'])==r['model_sha256']and sha(r['source_model'])==r['source_model_sha256']and r['source_worker_unchanged']
 def visit(v):
  if isinstance(v,dict):
   if 'path'in v and 'sha256'in v:
    rel=v['path'];p=LIB/rel
    if not p.is_file()or sha(p)!=v['sha256']:p=Path(fallback[rel])
    assert sha(p)==v['sha256'];row=dict(logical_path=rel,file=str(p),sha256=v['sha256']);assert rel not in resources or resources[rel]==row;resources[rel]=row
   for x in v.values():visit(x)
  elif isinstance(v,list):
   for x in v:visit(x)
 for record in records:
  source=SOURCE/record['contract'];assert sha(source)==record['sha256'];native=read(source)['native'];visit(native);patch=next(p for p in native['patch_states']if p['id']==record['id']);binding=next(b for b in plan['bindings']if b['id']==record['id']);changed=next(b for b in amended['bindings']if b['id']==record['id']);duration=sum(f['delay']+1 for f in patch['transition']);assert duration==binding['transition_duration'];terminal=max(1,duration-1);assert terminal==record['terminal_tick']==binding['terminal_tick'];physical={}
  for state,r in exports.items():
   endpoint=binding['endpoints'][state];position=endpoint['placement'];assert position['kind']=='blender-z-up-translation';x,y,z=position['value'];runtime={'id':record['id']+'-'+state,'role':'objects','model':state+'/model.glb','model_sha256':r['model_sha256'],'model_scene':r['model_scene'],'resources':[],'position':[x,z,-y]};physical[state]=[runtime]
   changed['endpoints'][state].update(kind='approved-private-gltf-browser-and-aperture-pending',model=pin(Path(r['source_model'])),appearance_user_approved=True,texture_completion_required=False,runtime_asset=runtime,appearance_approval={'receipt':pin(RECEIPT),'asset_id':'croisement02-hole-'+state,'scope':'texture'},export_report=pin(DEST/state/'export.json'))
  family=dict(id=record['id']+'-approved-hole',element_ids=[],background_ids=[],patch_ids=[record['id']],body_terminal_tick=terminal,physical=physical);contract=dict(version=1,scope='controlled-state-preview',native=native,families=[family]);file=DEST/(record['id']+'-delivery.json');write(file,contract);assert contract['native']==read(source)['native']
  rows.append(dict(id=record['id'],mission=record['mission'],contract=pin(file),source_contract=pin(source),terminal_tick=terminal,transition_duration=duration,physical=physical,aperture=binding['aperture'],aperture_runtime_included=False))
 for original,new in zip(plan['bindings'],amended['bindings']):
  assert original['id']==new['id']
  if original['id']not in hole_ids:assert original==new
  else:assert {k:v for k,v in original.items()if k!='endpoints'}=={k:v for k,v in new.items()if k!='endpoints'}
 amended.update(status='PRIVATE_HOLE_APPEARANCES_APPROVED_BROWSER_AND_CURRENT_APERTURES_PENDING',parent=pin(PLAN),publication_allowed=False);write(DEST/'bindings-private.json',amended)
 report=dict(status='PRIVATE_APPROVED_HOLE_CONTRACTS_BROWSER_AND_CURRENT_APERTURES_PENDING',approval=pin(RECEIPT),parent_bindings=pin(PLAN),bindings=rows,all_63_binding_ids_and_nonendpoint_fields_preserved=True,other_33_bindings_unchanged=True,models={s:pin(DEST/s/'model.glb')for s in exports},native_reader=dict(mapping=list(resources.values()),hash_checked=True,no_resource_rewrites=True),asset_handle=dict(root_directory=str(DEST),mapping={s+'/model.glb':str(DEST/s/'model.glb')for s in exports},read_only=True),recipe=pin(Path(__file__).resolve()),publication_allowed=False,scope='Private hole endpoints only; source-native transitions remain exact, and current receiver apertures must still be integrated before production use. All mound/climbing/scatter/orphan semantics remain unchanged.')
 write(DEST/'reader-config.json',report);print(json.dumps(dict(controls=len(rows),native_resources=len(resources),config=str(DEST/'reader-config.json'))))
if __name__=='__main__':main()
