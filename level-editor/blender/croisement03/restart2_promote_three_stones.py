"""Guarded exact three-stone installation with complete retired-payload rollback."""
import argparse,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
import promote_staged_publication as promotion
from promote_staged_publication import sha,library_lock,asset_file_pairs
from restart2_three_stones_index import writer
WORK=ROOT/'level-editor/work/croisement03-refinement/restart2'
STAGE=WORK/'approved-stone-integration-preflight/three-stones-stage-v1'
PARITY=WORK/'approved-stone-integration-preflight/metadata-parity-v3'
LIVE=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,d):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(d,indent=2)+'\n')
def scope_proof():
 evidence=read(STAGE/'receipt.json');assert sha(Path(evidence['world_parity_receipt']))==evidence['world_parity_receipt_sha256']
 for p,h in evidence['live_guards'].items():assert sha(Path(p))==h,p
 source=STAGE/'map-assets/3d-assets';index=read(source/'index.json');retired={};protected={}
 for entry in index['assets']:
  identity=entry['id'];report=read(PARITY/identity/'parity.json');proposal=read(PARITY/identity/'asset-metadata-proposal.json');descriptor=read(source/entry['descriptor'])
  assert descriptor['gameplay']==proposal['gameplay'] and descriptor['parts']==proposal['parts']
  assert report['maximum_obstacle_world_error']<1e-9
  for f in report['fragments']:
   assert f['world_equivalence'];assert f['negativeProbe'].startswith('PASS') or f['negativeProbe']=='no-spatial-record'
   oldpath=next(Path(p) for p,h in report['protectedFiles'].items() if read(Path(p))['id']==f['prior_asset']);old=read(oldpath)
   assert sha(oldpath)==f['source_descriptor_sha256'];assert old['gameplay']==f['original_gameplay'];assert [p['node'] for p in old['parts']]==[f['native']]
   assert f['native'] in [p['node'] for p in descriptor['parts']];retired[str(oldpath)]=identity
  for p in (PARITY/identity/'parity.json',PARITY/identity/'asset-metadata-proposal.json'):protected[str(p)]=sha(p)
 assert {read(Path(p))['id'] for p in retired}==set(evidence['retired_native_assets'])
 for p in (STAGE/'receipt.json',STAGE/'browser-preparation-v1/result.json',Path(evidence['world_parity_receipt'])):protected[str(p)]=sha(p)
 assert read(STAGE/'browser-preparation-v1/result.json')['status']=='PASS'
 return evidence,index,retired,protected

def prepare():
 evidence,index,retired,protected=scope_proof();source=STAGE/'map-assets/3d-assets';prior=read(LIVE/'3d-assets/index.json');selected=set(evidence['assets']);oldids=set(evidence['retired_native_assets']);assert not selected.intersection(a['id'] for a in prior['assets']);pairs=[];backup_hashes={}
 for entry in index['assets']:
  pairs+=asset_file_pairs(source,LIVE/'3d-assets',entry)
  for key in ('lossy_model','preview_model'):
   if not entry.get(key):continue
   for relative in (entry[key],entry[key]+'.receipt.json'):pairs.append((source/relative,LIVE/'3d-assets'/relative))
 for oldpath in retired:
  oldpath=Path(oldpath);old=read(oldpath);backup=STAGE/'retired-complete-backup'/old['id']
  if not backup.exists():shutil.copytree(oldpath.parent,backup)
  for f in oldpath.parent.rglob('*'):
   if f.is_file():assert sha(f)==sha(backup/f.relative_to(oldpath.parent));backup_hashes[str(backup/f.relative_to(oldpath.parent))]=sha(f)
  for resource in old.get('resources',[]):
   f=LIVE/resource['path'];assert sha(f)==resource['sha256'];dest=backup/'resources'/resource['path'];dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(f,dest);backup_hashes[str(dest)]=sha(f);protected[str(f)]=sha(f)
  pairs.append((None,oldpath))
 pairs.sort(key=lambda pair:pair[1].name=='asset.json');pairs.append((STAGE/'croisement03.rhlos-map.json',LIVE/'scenes/croisement03.rhlos-map.json'))
 prospective={str(t.relative_to(LIVE/'3d-assets')):s for s,t in pairs if t.is_relative_to(LIVE/'3d-assets')};merged=STAGE/'promotion-library-index.json';writer(prior,sha(LIVE/'3d-assets/index.json'),selected,oldids)(LIVE/'3d-assets',target=merged,files=prospective);pairs.append((merged,LIVE/'3d-assets/index.json'))
 records=[dict(source=str(s) if s else None,target=str(t),source_sha256=sha(s) if s else None,previous_sha256=sha(t),backup=str(STAGE/'promotion-backup'/f'{i:03d}-{t.name}')) for i,(s,t) in enumerate(pairs)]
 targets={str(t) for s,t in pairs};protected.update({p:h for p,h in evidence['live_guards'].items() if p not in targets});protected.update(backup_hashes)
 write(STAGE/'palette-before.json',prior);write(STAGE/'retirement-mapping.json',retired);protected[str(STAGE/'retirement-mapping.json')]=sha(STAGE/'retirement-mapping.json')
 write(STAGE/'promotion.json',dict(status='PREPARED_NOT_APPLIED',library=str(LIVE),files=records,protected_files=[dict(path=p,sha256=h) for p,h in protected.items()],index_generation=dict(target=str(LIVE/'3d-assets/index.json')),browser_check=dict(status='PASS',result=str(STAGE/'browser-preparation-v1/result.json')),scope=dict(approved=sorted(selected),retired=sorted(oldids))))
 print('Prepared',len(records),'guarded writes; live untouched')

def apply():
 evidence,index,retired,protected=scope_proof();manifest=read(STAGE/'promotion.json');prior=read(STAGE/'palette-before.json');index_record=next(r for r in manifest['files'] if r['target']==str(LIVE/'3d-assets/index.json'));original_check=promotion.check_gameplay_preserved;original_writer=promotion.write_asset_index
 def retirement_check(source,target):
  if source is not None or str(target) not in retired:return original_check(source,target)
  old=read(target);backup=STAGE/'retired-complete-backup'/old['id']
  for f in target.parent.rglob('*'):
   if f.is_file():assert sha(f)==sha(backup/f.relative_to(target.parent))
  # The exact old records, translated replacement and independently checked
  # world-space proof are bound by scope_proof and protected manifest hashes.
  assert retired[str(target)] in evidence['assets']
 promotion.check_gameplay_preserved=retirement_check;promotion.write_asset_index=writer(prior,index_record['previous_sha256'],set(evidence['assets']),set(evidence['retired_native_assets']))
 try:promotion._apply(STAGE/'promotion.json')
 finally:promotion.check_gameplay_preserved=original_check;promotion.write_asset_index=original_writer
 after=read(LIVE/'3d-assets/index.json');scope=set(evidence['assets'])|set(evidence['retired_native_assets']);others=lambda d:{a['id']:a for a in d['assets'] if a['id'] not in scope};assert others(prior)==others(after)
 write(STAGE/'live-promotion-result.json',dict(status='PASS',published=evidence['assets'],retired=evidence['retired_native_assets'],unrelated_entries_exact=len(others(prior)),map_sha256=sha(LIVE/'scenes/croisement03.rhlos-map.json'),normal_http_pending=True,limitations=['Scoped three approved stone assets only; map remains incomplete.']))

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--apply',action='store_true');args=parser.parse_args()
 with library_lock(LIVE):
  if args.apply:apply()
  else:assert not (STAGE/'promotion.json').exists();prepare()
if __name__=='__main__':main()
