"""Prepare one private production endpoint contract for proven absent initial artwork."""
import hashlib,json,shutil
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';DEST=BASE/'restart27-orphan-delivery-v1';LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return {'path':str(p),'sha256':sha(p)}
def read(p):return json.loads(Path(p).read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 plan=read(BASE/'restart25-approved-state-materialization-v1/approved-bindings.json');id='mission-Tac19_FoB_EC-patch-000';row=next(r for r in plan['bindings'] if r['id']==id);assert row['endpoints']['initial']['kind']=='source-absent'
 source=ROOT/row['source_contract']['path'];assert sha(source)==row['source_contract']['sha256'];native=read(source)['native'];patch=next(p for p in native['patch_states'] if p['id']==id)
 candidate=row['endpoints']['applied']['private_runtime_candidate'];model=Path(candidate['path']);assert sha(model)==candidate['sha256'];assert candidate['model_scene']=='scatter-site-13';assert row['terminal_tick']==92
 manifest=read(BASE/'restart7-source-patch-delivery/contracts-v1/manifest.json');fallback={r['path']:r['source'] for r in manifest['resources']};resources={}
 def visit(value):
  if isinstance(value,dict):
   if 'path' in value and 'sha256' in value:
    rel=value['path'];p=LIB/rel
    if not p.is_file() or sha(p)!=value['sha256']:p=Path(fallback[rel])
    assert sha(p)==value['sha256'];record={'logical_path':rel,'file':str(p),'sha256':value['sha256']}
    assert rel not in resources or resources[rel]==record;resources[rel]=record
   for v in value.values():visit(v)
  elif isinstance(value,list):
   for v in value:visit(v)
 visit(native)
 first=patch['initial'][0];im=Image.open(resources[first['path']]['file']).convert('RGBA');assert im.size==(4,1) and not any(im.getchannel('A').getdata())
 catalog=read(LIB/'mission-states/index.json');entry=next(e for e in catalog['entries'] if e['map']=='Croisement02' and e['mission']=='Tac19_FoB_EC')
 for k in ['mission_data','level_data']:assert sha(LIB/entry[k]['path'])==entry[k]['sha256']
 physical={'id':'private-orphan-scatter','role':'objects','model':'scatter.glb','model_sha256':candidate['sha256'],'model_scene':candidate['model_scene'],'resources':[],'position':[0,0,0]}
 contract={'version':1,'scope':'controlled-state-preview','native':native,'families':[{'id':'private-orphan-leaf-scatter','element_ids':[],'background_ids':[],'patch_ids':[id],'body_terminal_tick':92,'physical':{'initial':{'kind':'absent'},'applied':[physical]}}]}
 DEST.mkdir(exist_ok=False);bundle=DEST/'bundle';bundle.mkdir();shutil.copyfile(model,bundle/'scatter.glb');write(bundle/'delivery.json',contract)
 report={'status':'PRIVATE_ORPHAN_ONLY_PENDING_PRODUCTION_VALIDATION','identity':id,'mission':'Tac19_FoB_EC','family_id':'private-orphan-leaf-scatter','scene':'scatter-site-13','display_position':patch['display_position'],'terminal_tick':92,'initial_absence':{'source':first,'decoded_size':[4,1],'positive_alpha_pixels':0},'contract':pin(bundle/'delivery.json'),'source_contract':pin(source),'model':pin(bundle/'scatter.glb'),'catalog':pin(LIB/'mission-states/index.json'),'asset_handle':{'root_directory':str(bundle),'mapping':{'scatter.glb':str(bundle/'scatter.glb')},'read_only':True},'native_reader':{'mapping':list(resources.values()),'hash_checked':True,'no_resource_rewrites':True},'mission_source':{'name':'Tac19_FoB_EC','data':pin(LIB/entry['mission_data']['path']),'level':pin(LIB/entry['level_data']['path']),'camera':{'kind':'oblique-orthographic','elevation_deg':35}},'all_scenes_loader':{'source_report':pin(BASE/'restart25-approved-state-materialization-v1/scatter-export-v2/report.json'),'binding_template':dict(physical),'selection':'Use scene-specific ids and names scatter-site-00..19; saved-world placement, no extra translation; direct loader checks only'},'scope':'Only the proven source-absent orphan receives a full delivery contract. Other31 hiding controls have no fake absent initial and remain HOLD. Native contract/artwork/timing unchanged; no publication.'}
 write(DEST/'reader-config.json',report);print(json.dumps({'config':str(DEST/'reader-config.json'),'resources':len(resources),'contract_sha256':sha(bundle/'delivery.json')}))
if __name__=='__main__':main()
