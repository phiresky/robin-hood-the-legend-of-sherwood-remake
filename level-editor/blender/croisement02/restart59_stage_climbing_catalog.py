"""Stage two approved climbing controls for normal editor review, without publication."""
import hashlib,json,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
LIB=ROOT/'level-editor/library'
BASE=ROOT/'level-editor/work/croisement02-refinement'
SOURCE=BASE/'restart28-approved-climbing-integration-v3'
OUT=BASE/'restart59-climbing-catalog-stage-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def pin(p):return {'path':str(p),'sha256':sha(p)}
def check(p):assert sha(Path(p['path']))==p['sha256'];return Path(p['path'])
def save(p,d):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert not OUT.exists();OUT.mkdir()
 cfg=read(SOURCE/'reader-config.json');check(cfg['approval']);proof=BASE/'restart52-climbing-browser-v1/handoff.json';d=read(proof);assert d['status']=='PASS_SCOPED_CLIMBING_ENDPOINTS_AND_NATIVE_CONTROLS'
 rows=[];prefix='mission-states/croisement02/reviewed-climbing-v1'
 for phase,p in cfg['models'].items():
  source=check(p);relative=f'{prefix}/{phase}/model.glb';target=OUT/'library'/relative;target.parent.mkdir(parents=True);shutil.copyfile(source,target);rows.append({'destination':relative,'staged':pin(target),'source':p})
 before=read(LIB/'mission-states/index.json');entries=[]
 for row in cfg['bindings']:
  contract=read(check(row['contract']))
  for family in contract['families']:
   for phase in ['initial','applied']:
    for binding in family['physical'][phase]:binding['model']=prefix+'/'+binding['model']
  relative=f"{prefix}/contracts/{row['id']}.json";p=OUT/'library'/relative;save(p,contract);rows.append({'destination':relative,'staged':pin(p),'source':row['contract']})
  template=next(e for e in before['entries']if e['mission']==row['mission']and e['map']=='Croisement02')
  entries.append({'id':'reviewed-climbing-'+row['id'],'name':'Climbing vegetation','kind':'transition','map':'Croisement02','mission':row['mission'],'contract':{'path':relative,'sha256':sha(p)},'mission_data':template['mission_data'],'level_data':template['level_data']})
 assert len(entries)==2 and not {e['id']for e in entries}&{e['id']for e in before['entries']}
 p=OUT/'library/mission-states/index.json';save(p,{**before,'entries':before['entries']+entries});rows.append({'destination':'mission-states/index.json','staged':pin(p)})
 plan={'status':'PRIVATE_CATALOG_UI_REVIEW_PENDING','files':rows,'catalog_additions':entries,'catalog_before':pin(LIB/'mission-states/index.json'),'approval':cfg['approval'],'endpoint_browser':pin(proof),'source_config':pin(SOURCE/'reader-config.json'),'native_transition':'Exact source artwork; terminal tick5; definitive reset. No physical interpolation or static replacement inferred.','limitations':d['limitations'],'canonical_writes':False};save(OUT/'plan.json',plan)
 save(OUT/'browser-request.json',{'status':'QUEUED_NORMAL_EDITOR_CATALOG_REVIEW','plan':pin(OUT/'plan.json'),'routes':{r['destination']:r['staged']for r in rows},'entries':entries,'checks':['Actual map and mission selectors load both new catalog entries','Initial/applied endpoints match approved hashes and native transition preserves tick5 terminal/background','Physical/native/reset/mission switch/map retirement leave unrelated state unchanged'],'prior_appearance_proof':pin(proof),'canonical_writes':False})
 print(json.dumps(pin(OUT/'plan.json')))
if __name__=='__main__':main()
