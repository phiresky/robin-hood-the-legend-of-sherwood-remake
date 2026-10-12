"""Prepare a private browser candidate; preserve all prior evidence and library blobs."""
import copy,hashlib,json,os,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import editor_descriptor
BASE=ROOT/'level-editor/work/active/croisement03/restart2'
STAGE=BASE/'combined-trio-stage-v2'
SOURCE=STAGE/'browser-library-overlay-v2'
CANDIDATE=BASE/'combined-trio-inferred-crown-candidate-v1'
OUT=CANDIDATE/'browser-library-v1'
LIB=OUT/'library'

def sha(p):
 with Path(p).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def read(p):return json.loads(Path(p).read_text())
def write(p,d):p.write_text(json.dumps(d,separators=(',',':'))+'\n')
def main():
 assert not OUT.exists()
 assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))>=6*1024**2
 receipt=read(CANDIDATE/'receipt.json');probe=read(CANDIDATE/'native-overlap-conservative-v3.json')
 assert probe['status']=='NO_CONSERVATIVE_NATIVE_FRONT_OVERLAP' and all(r['in_map_opaque_pixels']==0 and r['above_map_opaque_pixels']>0 for r in probe['rows'])
 request=read(SOURCE/'browser-request.json');source_lib=SOURCE/'library'
 document=read(request['map']);original=copy.deepcopy(document)
 replacements={'scenes/croisement03.rhlos-map.json','3d-assets/index.json'}
 descriptors={}
 for row in receipt['rows']:
  n=row['tree'];identity=f'croisement03-tree-{n}';source=CANDIDATE/f'tree{n}'
  assert sha(source/'model.glb')==row['candidate_sha256'] and sha(source/'asset.json')==row['descriptor_sha256']
  ref=next(r for r in document['assetSources']if r['id']==identity)
  for field in ('model','descriptor'):
   replacements.add(ref[field]);ref[field+'_sha256']=sha(source/('model.glb'if field=='model'else'asset.json'))
  descriptors[identity]=read(source/'asset.json')
 assert document['placements']==original['placements'] and document['sceneMetadata']==original['sceneMetadata']
 OUT.mkdir();LIB.mkdir()
 def link(source,target):
  rel=source.relative_to(source_lib).as_posix()
  if rel in replacements:return
  if source.is_symlink():assert rel=='game-data';target.symlink_to(source.resolve(),target_is_directory=True)
  elif source.is_dir():
   target.mkdir()
   for child in source.iterdir():link(child,target/child.name)
  else:os.link(source,target)
 for p in source_lib.iterdir():link(p,LIB/p.name)
 write(LIB/'scenes/croisement03.rhlos-map.json',document)
 for row in receipt['rows']:
  ref=next(r for r in document['assetSources']if r['id']==f'croisement03-tree-{row["tree"]}')
  for field,name in [('model','model.glb'),('descriptor','asset.json')]:os.link(CANDIDATE/f'tree{row["tree"]}'/name,LIB/ref[field])
 index=read(source_lib/'3d-assets/index.json');changed=[]
 for row in index['assets']:
  if row['id']not in descriptors:continue
  descriptor=descriptors[row['id']];row['editor']=editor_descriptor(descriptor);row['descriptor_sha256']=sha(LIB/'3d-assets'/row['descriptor']);assert 'lossy_model'not in row and 'preview_model'not in row;changed.append(row['id'])
 assert sorted(changed)==sorted(descriptors)
 write(LIB/'3d-assets/index.json',index)
 pins={}
 for name,digest in request['pins'].items():
  old=Path(name)
  try:
   relative=old.relative_to(source_lib);new=LIB/relative
  except ValueError:new=old;relative=None
  actual=sha(new)
  if str(relative)not in replacements:assert actual==digest,str(new)
  pins[str(new)]=actual
 for name in ('receipt.json','native-overlap-conservative-v3.json'):pins[str(CANDIDATE/name)]=sha(CANDIDATE/name)
 request.update(status='HOLD appearance candidate; ready for explicit14phase/source/oblique browser verification',library=str(LIB),map=str(LIB/'scenes/croisement03.rhlos-map.json'),pins=pins,catalog_overlay=str(OUT/'receipt.json'),scope='Previous exact combined trio plus inferred-only out-of-map crown context for Tree12/14; zero observed frame0 proxy faces restored. New combined appearance unapproved.')
 write(OUT/'browser-request.json',request)
 fixture=CANDIDATE/'browser-check-fixture-v1';fixture.mkdir();oldfixture=STAGE/'browser-check-fixture-v2';cfg=read(oldfixture/'config.json');cfg.update(library=str(LIB),map=request['map'],mapSha256=sha(request['map']))
 for ref in document['assetSources']:
  if ref['id']in descriptors:cfg['assets'][ref['id']]=ref
 write(fixture/'config.json',cfg)
 # Correct the proven terrain-reference lookup; all other source assertions remain unchanged.
 check=(oldfixture/'check.js').read_text().replace('initial.assetSources.find(x => x.id === id)','[...(initial.assetSources ?? []), ...(initial.sceneAssets ?? [])].find(x => x.id === id)')
 (fixture/'check.js').write_text(check)
 plan=read(oldfixture/'hook-plan.json');plan['browserRequest']=str(OUT/'browser-request.json');plan['pins']={str(fixture/n):sha(fixture/n)for n in ('check.js','config.json')};write(fixture/'hook-plan.json',plan)
 report={'status':'CPU_PRIVATE_STAGE_READY_BROWSER_APPEARANCE_HOLD','changed_assets':changed,'placements_exact':True,'scene_metadata_exact':True,'unchanged_request_pins_exact':True,'map_sha256':sha(request['map']),'catalog_sha256':sha(LIB/'3d-assets/index.json'),'browser_request':str(OUT/'browser-request.json'),'fixture':str(fixture),'canonical_written':False,'limitations':['CPU alpha projection is independent evidence; actual renderer first-hit/source and oblique visual verification still required.','No existing approval inferred for changed combined appearance. Hardlinked library files are read-only.']}
 write(OUT/'receipt.json',report);print(json.dumps(report))
if __name__=='__main__':main()
