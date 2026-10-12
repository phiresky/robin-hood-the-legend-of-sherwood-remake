"""Stage additive reviewed hole/mound catalog entries without publishing library assets."""
import copy
import hashlib
import json
import os
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
LIB=ROOT/'level-editor/library'
BASE=ROOT/'level-editor/work/active/croisement02'
OUT=BASE/'restart53-hole-mound-transaction-v1'
STAGE=OUT/'library'

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def pin(p):return {'path':str(p),'sha256':sha(p)}
def load(p):return json.loads(p.read_text())
def save(p,d):
 p.parent.mkdir(parents=True,exist_ok=True);assert not p.exists();p.write_text(json.dumps(d,indent=2)+'\n')
files={}
def model(source,dest):
 source=Path(source);target=STAGE/dest;target.parent.mkdir(parents=True,exist_ok=True);assert not target.exists();os.link(source,target);files[dest]={'source':pin(source),'staged':pin(target),'destination':dest,'expected_before':sha(LIB/dest)if(LIB/dest).exists()else None};return dest

def written(dest,d):
 p=STAGE/dest;save(p,d);files[dest]={'staged':pin(p),'destination':dest,'expected_before':sha(LIB/dest)if(LIB/dest).exists()else None};return {'path':dest,'sha256':sha(p)}
assert not OUT.exists()
index=load(LIB/'mission-states/index.json');original=copy.deepcopy(index['entries']);entries=[]
integration=load(BASE/'restart47-explicit-rgb-sampling-v1/holes/integration-focused.json')
configs={kind:load(BASE/f'restart47-explicit-rgb-sampling-v1/{kind}/reader-config.json')for kind in ['holes','mounds']}
routes={}
for kind,cfg in configs.items():
 for logical,source in cfg['asset_handle']['mapping'].items():
  dest=f'mission-states/croisement02/reviewed-{kind}-v1/'+logical
  routes[(kind,logical)]=model(source,dest)
manifest={'version':1,'map':'Croisement02','receivers':[],'endpoints':{},'sites':[{k:s[k]for k in ['id','patches','endpoint_position']}for s in integration['sites']],'controls':[{k:c[k]for k in ['id','mission','family','patches','site']}for c in integration['controls']]}
for r in integration['receivers']:
 descriptor=load(Path(r['descriptor']['path']));group=None if r['asset']=='croisement02-terrain'else r['asset'];dest=model(r['model']['path'],f"mission-states/croisement02/reviewed-holes-v1/{r['asset']}-apertures.glb")
 manifest['receivers'].append({'asset':r['asset'],**({'group':group}if group else{}),'model_sha256':r['source']['sha256'],'descriptor_sha256':r['descriptor']['sha256'],'placement':r['placement'],'parts':[f"asset:{r['asset']}:{p['node']}"for p in descriptor.get('parts',[])]if group else[],'replacement':{'id':r['asset']+'-apertures','role':'objects','model':dest,'model_sha256':r['model']['sha256'],'model_scene':r['partition_scene'],'resources':[]}})
for phase in ['initial','applied']:
 src=integration['endpoints'][phase];manifest['endpoints'][phase]={'id':'hole-'+phase,'role':'objects','model':routes[('holes',phase+'/model.glb')],'model_sha256':src['model']['sha256'],'model_scene':src['scene'],'resources':[]}
aperture=written('mission-states/croisement02/reviewed-holes-v1/apertures.json',manifest)
for kind,cfg in configs.items():
 for row in cfg['bindings']:
  contract=load(Path(row['contract']['path']));assert sha(Path(row['contract']['path']))==row['contract']['sha256']
  for family in contract['families']:
   for phase in ['initial','applied']:
    if isinstance(family['physical'][phase],list):
     for binding in family['physical'][phase]:binding['model']=routes[(kind,binding['model'])]
  contractpin=written(f"mission-states/croisement02/reviewed-{kind}-v1/contracts/{row['id']}.json",contract)
  template=next(e for e in original if e['mission']==row['mission']and e['map']=='Croisement02')
  entry={'id':'reviewed-'+kind+'-'+row['id'],'name':('Leaf-covered hole'if kind=='holes'else 'Hiding mound')+' — '+row['id'].rsplit('-',1)[-1],'kind':'transition','map':'Croisement02','mission':row['mission'],'contract':contractpin,'mission_data':template['mission_data'],'level_data':template['level_data']}
  if kind=='holes':entry['aperture']=aperture
  assert not any(e['id']==entry['id']for e in original+entries);entries.append(entry)
index['entries']=original+entries;written('mission-states/index.json',index)
proofs=[BASE/'restart46-hole-context-browser-v3/handoff.json',BASE/'restart49-mounds-isolated-browser-v1/handoff.json',BASE/'restart51-mound-lifecycle-browser-v2/handoff.json']
transaction={'status':'PRIVATE_ADDITIVE_CATALOG_TRANSACTION_EDITOR_WIRING_BROWSER_PENDING','files':list(files.values()),'catalog_before':pin(LIB/'mission-states/index.json'),'catalog_additions':entries,'unrelated_entries_preserved':len(original),'holes':30,'mounds':32,'hole_sites':17,'mound_sites':20,'map_source_guard':integration['map'],'receiver_source_guards':[{'model':r['source'],'descriptor':r['descriptor']}for r in integration['receivers']],'approvals':[configs[k]['approval']for k in configs],'browser_proofs':[pin(p)for p in proofs],'runtime_commit':'cb8686dcb','recipe':pin(Path(__file__)),'canonical_writes':False,'root_apply_only':True,'limitations':['New catalog-to-viewport aperture loading needs full editor browser proof before publication.','Mound applied scatter capture framing is limited; lifecycle/native restoration passed, but full scatter appearance has separate approval evidence.','Merge catalog_additions against current catalog if another session adds entries; never overwrite unrelated catalog changes.']}
save(OUT/'transaction.json',transaction);print(json.dumps(pin(OUT/'transaction.json')))
