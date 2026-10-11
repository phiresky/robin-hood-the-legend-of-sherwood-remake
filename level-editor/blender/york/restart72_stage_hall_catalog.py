"""Stage approved hall source-loop resources and one rebased normal catalog entry."""
import hashlib,json,os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';O=W/'restart72-hall-normal-catalog-stage-v1';LIB=ROOT/'level-editor/library';S=W/'restart70-hall-visibility-private-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not O.exists(),'Fresh stage required'
 c=json.loads((S/'contract.proposed.json').read_text());assert len(c['cover_visibility'])==1 and len(c['native']['cover_artwork']['variants'])==4
 baseline=LIB/'mission-states/index.json';baseline_sha=sha(baseline);catalog=json.loads(baseline.read_text());entry=json.loads((W/'restart69-hall-approved-delivery-v1/catalog-addition.pending.json').read_text());assert all(e['id']!=entry['id'] for e in catalog['entries'])
 resources={}
 def collect(x):
  if isinstance(x,dict):
   if all(k in x for k in ('path','sha256')):resources[x['path']]=x['sha256']
   if 'model' in x and 'model_sha256' in x:resources[x['model']]=x['model_sha256']
   for v in x.values():collect(v)
  elif isinstance(x,list):
   for v in x:collect(v)
 collect(c);collect(entry['mission_data']);collect(entry['level_data'])
 records=[]
 for rel,digest in sorted(resources.items()):
  assert Path(rel).is_relative_to('mission-states/york');src=S/'library'/rel;assert sha(src)==digest,rel;dest=O/'library'/rel;dest.parent.mkdir(parents=True,exist_ok=True);os.link(src,dest);records.append({'relative':rel,'source':str(dest),'sha256':digest,'bytes':dest.stat().st_size})
 dest=O/'library'/entry['contract']['path'];dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes((S/'contract.proposed.json').read_bytes());entry['contract']['sha256']=sha(dest);records.append({'relative':entry['contract']['path'],'source':str(dest),'sha256':sha(dest),'bytes':dest.stat().st_size})
 catalog['entries'].append(entry);dest=O/'library/mission-states/index.json';dest.write_text(json.dumps(catalog,separators=(',',':'))+'\n');records.append({'relative':'mission-states/index.json','source':str(dest),'sha256':sha(dest),'bytes':dest.stat().st_size});assert sha(baseline)==baseline_sha
 views_path=W/'hall-textures-v2/applied-applied/experiment/views.json';views=json.loads(views_path.read_text());(O/'saved-hall-views.json').write_text(json.dumps(views,indent=2)+'\n')
 plan={'status':'COMPLETE_HALL_CATALOG_STAGE_PENDING_NORMAL_EDITOR_PROOF','entry':entry,'baseline_catalog_sha256':baseline_sha,'records':records,'approved_flame_sha256':'3edab8b9da5fd995205224e956c5a76d716eaaa49d7d3b3380df04e0dbf99eba','contract_sha256':entry['contract']['sha256'],'source_views':{'path':str(views_path),'sha256':sha(views_path)},'publication':'Root only; rebase this single entry under publication lock. No ungated restart69 contract. Other existing catalog resources remain canonical; stage is an overlay.','canonical_writes':False};(O/'plan.json').write_text(json.dumps(plan,indent=2)+'\n');print(json.dumps({'stage':str(O),'records':len(records),'plan_sha256':sha(O/'plan.json')}))
if __name__=='__main__':main()
