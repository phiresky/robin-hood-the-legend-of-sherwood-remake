"""Rebase cached receiver pieces only when changed models own no cached work."""
import argparse,json,hashlib,copy
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];LIB=ROOT/'level-editor/library'
sha=lambda p:hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--checkpoint',type=Path,required=True);p.add_argument('--old-map',type=Path,required=True);p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists();cache=a.output.with_suffix('.checkpoint.json');assert not cache.exists();saved=json.loads(a.checkpoint.read_text());old=json.loads(a.old_map.read_text());current=LIB/'scenes/croisement02.rhlos-map.json';new=json.loads(current.read_text());assert sha(a.old_map)==saved['pins']['map_sha256'];assert sha(a.candidate/'fit.json')==saved['pins']['fit_sha256'];assert sha(a.candidate/'geometry.py')==saved['pins']['geometry_sha256'];assert sha(Path(__file__).with_name('restart94_piece_envelope_resume.py'))==saved['pins']['recipe_sha256']
 # Geometry traversal and ordering must be identical. Descriptor metadata does not enter triangle evaluation.
 stripped=copy.deepcopy(new);changes=[]
 assert len(old['assetSources'])==len(new['assetSources'])
 for i,(before,after) in enumerate(zip(old['assetSources'],new['assetSources'])):
  assert before['id']==after['id']
  for key in ['model_sha256','descriptor_sha256']:
   if before.get(key)!=after.get(key):changes.append({'id':before['id'],'field':key,'old':before.get(key),'new':after.get(key)});stripped['assetSources'][i][key]=before.get(key)
 assert stripped==old,'Unexpected placement, topology or source traversal change'
 changed_ids={r['id'] for r in changes if r['field']=='model_sha256'};cached_owners={owner for unit in saved['units'] for owner in unit['bands']};assert changed_ids.isdisjoint(cached_owners),'Changed geometry has cached exclusion intervals'
 for key in [*saved['progress']['completed'],saved['progress']['current']]:
  if key is not None:assert key.split(':')[0] not in changed_ids
 actual_changed=[]
 for name,before in list(saved['pins']['models'].items()):
  after=sha(Path(name));saved['pins']['models'][name]=after
  if after!=before:actual_changed.append(name)
 expected={str(LIB/r['model']) for r in new['assetSources'] if r['id'] in changed_ids};assert set(actual_changed)==expected,'Other receiver resource drift'
 for r in new['assetSources']:
  assert not r.get('model_sha256') or sha(LIB/r['model'])==r['model_sha256']
 saved['pins']['map_sha256']=sha(current);cache.write_text(json.dumps(saved,separators=(',',':'))+'\n');report={'status':'SAFE_PRIVATE_CHECKPOINT_REBASE_NOT_CLEARANCE_PROOF','old_checkpoint_sha256':sha(a.checkpoint),'new_checkpoint_sha256':sha(cache),'old_map_sha256':sha(a.old_map),'new_map_sha256':sha(current),'changes':changes,'changed_models':actual_changed,'cached_owners':sorted(cached_owners),'progress':saved['progress'],'guard':'No changed model owns cached work. Traversal and every placement unchanged. Reader will still reject unexpected newly eligible primitives before current checkpoint.'};a.output.with_suffix('.rebase.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
