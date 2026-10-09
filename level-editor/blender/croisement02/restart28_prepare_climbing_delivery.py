"""Bind the two approved climbing endpoints to unchanged native patch contracts."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';DEST=BASE/'restart28-approved-climbing-integration-v3';LIB=ROOT/'level-editor/library';SOURCE=BASE/'restart7-source-patch-delivery/contracts-v1'
APPROVAL=BASE/'restart3-review-batches/next-six-textures-channels-v1/user-approval.json';APPROVAL_SHA='7f6abc55fa21eee6ebd3b10c4f079e91e7f8159127a30a23fb9a6955d52e3f9d'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return {'path':str(p),'sha256':sha(p)}
def read(p):return json.loads(Path(p).read_text())
def write(p,d):assert not p.exists();p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert sha(APPROVAL)==APPROVAL_SHA
 manifest=read(SOURCE/'manifest.json');fallback={r['path']:r['source'] for r in manifest['resources']};records=[r for r in manifest['records'] if r['profile']=='Croisement02 - hidden archer05'];assert len(records)==2
 exports={s:read(DEST/s/'export.json') for s in ['initial','applied']}
 for state,export in exports.items():
  assert sha(export['model'])==export['model_sha256'];assert sha(export['source_model'])==export['source_model_sha256'];assert export['geometry_uv_ownership_alpha_preserved']
 resources={};rows=[];catalog=read(LIB/'mission-states/index.json')
 def visit(value):
  if isinstance(value,dict):
   if 'path' in value and 'sha256' in value:
    rel=value['path'];p=LIB/rel
    if not p.is_file() or sha(p)!=value['sha256']:p=Path(fallback[rel])
    assert sha(p)==value['sha256'];row={'logical_path':rel,'file':str(p),'sha256':value['sha256']};assert rel not in resources or resources[rel]==row;resources[rel]=row
   for item in value.values():visit(item)
  elif isinstance(value,list):
   for item in value:visit(item)
 for record in records:
  source=SOURCE/record['contract'];assert sha(source)==record['sha256'];original=read(source);native=original['native'];patch=next(p for p in native['patch_states'] if p['id']==record['id']);visit(native)
  assert patch['display_position']==[99,0] and patch['integrate_in_background'] and patch['definitive'] and not patch['final']
  duration=sum(f['delay']+1 for f in patch['transition']);terminal=max(1,duration-1);assert terminal==5==record['terminal_tick'];physical={}
  for state in ['initial','applied']:
   export=exports[state];construction=read(Path(export['source_model']).with_name('construction.json'));frame=patch['initial'][0] if state=='initial' else patch['transition'][-1]
   assert construction['source_sha256']==frame['sha256'];assert construction['source_top_left']==[p+o for p,o in zip(patch['display_position'],frame['offset'])]
   physical[state]=[{'id':'climbing-archer05-'+state,'role':'objects','model':state+'/model.glb','model_sha256':export['model_sha256'],'model_scene':export['model_scene'],'resources':[],'position':[0,0,0]}]
  family={'id':'climbing-archer05','element_ids':[],'background_ids':[],'patch_ids':[record['id']],'body_terminal_tick':terminal,'physical':physical}
  contract={'version':1,'scope':'controlled-state-preview','native':native,'families':[family]};path=DEST/(record['mission']+'-delivery.json');write(path,contract)
  entry=next(e for e in catalog['entries'] if e['map']=='Croisement02' and e['mission']==record['mission']);mission=LIB/entry['mission_data']['path'];level=LIB/entry['level_data']['path']
  for key,p in [('mission_data',mission),('level_data',level)]:assert sha(p)==entry[key]['sha256']
  raw=read(mission)['mission_patches'][record['index']]
  rows.append({'id':record['id'],'focus_patch_id':record['id'],'mission':record['mission'],'contract':pin(path),'source_contract':pin(source),'source_patch_index':record['index'],'terminal_tick':terminal,'transition_duration':duration,'initially_active':raw['active'],'definitive':raw['definitive'],'display_position':patch['display_position'],'saved_world_placement':[0,0,0],'endpoints':physical,'mission_source':{'name':record['mission'],'data':pin(mission),'level':pin(level),'camera':{'kind':'oblique-orthographic','elevation_deg':35}},'physical_transition':'Native artwork only; no inferred geometry interpolation','aperture':None})
 report={'status':'PRIVATE_APPROVED_CLIMBING_CONTRACTS_PENDING_CPU_AND_BROWSER','approval':pin(APPROVAL),'source_manifest':pin(SOURCE/'manifest.json'),'source_catalog':pin(LIB/'mission-states/index.json'),'current_map':pin(LIB/'scenes/croisement02.rhlos-map.json'),'models':{s:pin(DEST/s/'model.glb') for s in exports},'export_reports':{s:pin(DEST/s/'export.json') for s in exports},'bindings':rows,'native_reader':{'mapping':list(resources.values()),'hash_checked':True,'no_resource_rewrites':True},'asset_handle':{'root_directory':str(DEST),'mapping':{s+'/model.glb':str(DEST/s/'model.glb') for s in exports},'read_only':True},'recipe':pin(Path(__file__).resolve()),'publication_allowed':False,'scope':'Only climbing profile05 endpoint delivery. Native source controls, phases, background integration and reset remain unchanged. Other hidden-archer profiles, actors, static substrate ownership and canonical publication remain separate.'}
 write(DEST/'reader-config.json',report);print(json.dumps({'config':str(DEST/'reader-config.json'),'sha256':sha(DEST/'reader-config.json'),'controls':len(rows),'native_resources':len(resources)}))
if __name__=='__main__':main()
