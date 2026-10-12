"""Create a complete private read-through catalog for the exact staged C3 map."""
import copy,hashlib,json,sys
from pathlib import Path
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement'))
from asset_index import editor_descriptor
S=R/'level-editor/work/active/croisement03/restart2/combined-trio-stage-v2';LIVE=R/'level-editor/library';OUT=S/'browser-library-overlay-v1';LIB=OUT/'library'
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def read(p):return json.loads(Path(p).read_bytes())
def write(p,d):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(d,separators=(',',':'),ensure_ascii=False)+'\n')
def main():
 assert not OUT.exists();request=read(S/'browser-request.json');stage=S/'library';scene=stage/'scenes/croisement03.rhlos-map.json';document=read(scene);indexpath=LIVE/'3d-assets/index.json';original_index_hash=sha(indexpath);index=read(indexpath);refs=document['assetSources']+document['sceneAssets'];byid={e['id']:e for e in index['assets']};assert len(byid)==len(index['assets']);affected={r['id'] for r in refs};retired=set(read(S/'staging-receipt.json')['retired_assets']);omitted=set();replacement={};pins={str(scene):sha(scene),str(indexpath):original_index_hash,str(S/'browser-request.json'):sha(S/'browser-request.json')}
 for p,d in request['pins'].items():assert sha(p)==d,p;pins[p]=d
 for ref in refs:
  dp=stage/ref['descriptor'];mp=stage/ref['model'];assert sha(dp)==ref['descriptor_sha256'] and sha(mp)==ref['model_sha256'];descriptor=read(dp);assert descriptor['id']==ref['id'];relative=Path(ref['descriptor']).relative_to('3d-assets').as_posix();parent=Path(relative).parent
  row={k:descriptor[k] for k in ('id','name','source_map')};row.update(descriptor=relative,model=(parent/descriptor['model']).as_posix(),descriptor_sha256=sha(dp),editor=editor_descriptor(descriptor))
  for k in ('model_scene','editor_usage','asset_type','tags'):
   if k in descriptor:row[k]=descriptor[k]
  assert row['model']==Path(ref['model']).relative_to('3d-assets').as_posix();replacement[ref['id']]=row
  previous=byid.get(ref['id'],{})
  for field in ('lossy_model','preview_model'):
   if field in previous:omitted.update(['3d-assets/'+previous[field],'3d-assets/'+previous[field]+'.receipt.json'])
  for stem in ['lossy.glb','preview.glb','model.lossy.glb','model.preview.glb']:
   omitted.update([str(Path(ref['descriptor']).parent/stem),str(Path(ref['descriptor']).parent/(stem+'.receipt.json'))])
 result={k:v for k,v in index.items() if k!='assets'};result['assets']=[replacement.get(row['id'],row) for row in index['assets'] if row['id'] not in retired];result['assets'] += [row for identity,row in replacement.items() if identity not in byid];result['assets'].sort(key=lambda r:r['id']);assert len({r['id'] for r in result['assets']})==len(result['assets'])
 unchanged=0
 for row in result['assets']:
  if row['id'] not in affected:assert row==byid[row['id']];unchanged+=1
  else:assert 'lossy_model' not in row and 'preview_model' not in row
 OUT.mkdir();links={}
 def overlay(relative):
  if relative in omitted or relative=='3d-assets/index.json':return
  hi=stage/relative;lo=LIVE/relative;dest=LIB/relative
  if hi.exists() and hi.is_dir() and not hi.is_symlink():
   dest.mkdir(parents=True,exist_ok=True);names={p.name for p in hi.iterdir()}
   if lo.is_dir():names|={p.name for p in lo.iterdir()}
   for name in sorted(names):overlay(str(Path(relative)/name))
  else:
   source=hi if hi.exists() else lo
   assert source.exists(),str(source);dest.parent.mkdir(parents=True,exist_ok=True);dest.symlink_to(source.resolve(),target_is_directory=source.is_dir());links[relative]=str(source.resolve())
 overlay('');write(LIB/'3d-assets/index.json',result)
 for ref in refs:
  for field in ('descriptor','model'):assert sha(LIB/ref[field])==ref[field+'_sha256']
 assert sha(LIB/'scenes/croisement03.rhlos-map.json')==sha(scene);assert sha(indexpath)==original_index_hash
 pinmap={}
 for p,d in pins.items():
  source=Path(p)
  try:rel=source.relative_to(stage);target=LIB/rel
  except ValueError:target=source
  assert sha(target)==d;pinmap[str(target)]=d
 pinmap[str(LIB/'3d-assets/index.json')]=sha(LIB/'3d-assets/index.json')
 report=dict(status='PASS complete private catalog overlay; browser not launched',library=str(LIB),map=str(LIB/'scenes/croisement03.rhlos-map.json'),index=str(LIB/'3d-assets/index.json'),map_sha256=sha(scene),index_sha256=sha(LIB/'3d-assets/index.json'),source_catalog_sha256=original_index_hash,source_catalog_bytes=indexpath.stat().st_size,catalog_rows=len(result['assets']),unchanged_catalog_rows=unchanged,refreshed_map_rows=len(replacement),retired_catalog_rows=sorted(retired),omitted_derivative_paths=sorted(omitted),readthrough_paths=links,pins=pinmap,scope=request['scope'],required_views=request['required_views'],canonical_changed=False)
 write(OUT/'overlay-receipt.json',report);fresh=copy.deepcopy(request);fresh.update(status='READY exact staged map with complete private catalog overlay; browser validation pending',library=str(LIB),map=report['map'],pins=pinmap,catalog_overlay=str(OUT/'overlay-receipt.json'));write(OUT/'browser-request.json',fresh);print(json.dumps({k:report[k] for k in ['status','library','map_sha256','index_sha256','catalog_rows','unchanged_catalog_rows','refreshed_map_rows']}))
if __name__=='__main__':main()
