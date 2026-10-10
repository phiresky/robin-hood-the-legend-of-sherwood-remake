"""Freeze the private combined scene and describe guarded root-owned publication steps."""
import hashlib,json
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2/combined-trio-stage-v2';L=B/'library';CAN=R/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 check=read(B/'compile-validation.json');assert check['status'].startswith('PASS');m=L/'scenes/croisement03.rhlos-map.json';assert sha(m)==check['mapSha256'];doc=read(m);pins={str(m):sha(m)};writes=[]
 for ref in doc['assetSources']+doc['sceneAssets']:
  for field in ['descriptor','model']:
   p=L/ref[field];assert sha(p)==ref[field+'_sha256'];pins[str(p)]=sha(p)
  a=read(L/ref['descriptor'])
  for r in a.get('resources',[]):
   p=L/r['path'];assert sha(p)==r['sha256'];pins[str(p)]=sha(p)
 for p in L.rglob('*'):
  if not p.is_file() or p.is_symlink() or p.resolve().is_relative_to(CAN):continue
  rel=p.relative_to(L);dest=CAN/rel
  writes.append(dict(source=str(p),destination=str(dest),source_sha256=sha(p),expected_destination_sha256=sha(dest) if dest.exists() else None))
 for name in ['animation-contract.json','tree13-static-ownership.json','draft-normalization.json','compile-validation.json','staging-receipt.json']:
  p=B/name;pins[str(p)]=sha(p)
 st=read(B/'staging-receipt.json');assert all(sha(Path(p))==h for p,h in st['pins'].items())
 request=dict(status='READY FOR JOINT PRIVATE RUNTIME REVIEW; not publication approval',library=str(L),map=str(m),pins=pins,scope='Static-only Tree12/14; approved static Tree13; approved ground fill; one complete native Arbre06 animation. Tree10/11 remain current canonical placements.',required_views=['Original camera and all 14 animation phases over a complete 56-tick loop','Oblique views covering the three static crowns and native sprite plane','First-hit/ownership comparisons for Tree12/13/14 static domains and shared animated pixels','Fern/reserved domain crossings; no doubled frame0 proxies, missing crowns or restored painted duplicates'],evidence=[str(B/'tree13-static-ownership.json'),str(B/'animation-contract.json'),str(B/'compile-validation.json')],limitations=['Existing library index is a read-through symlink; do not write derivatives or index through it.','Pass on isolated full exports does not establish combined static/native runtime correctness.','Current recovered gameplay retains all distinct draft warnings.'])
 write(B/'browser-request.json',request)
 plan=dict(status='GUARDED PRIVATE PUBLICATION STAGING; root promotion disabled until runtime and derivative gates pass',writes=writes,retire_map_references=st['retired_assets'],delete_files=[],preconditions=['Verify all source pins and expected destination hashes immediately before changes','Root reviews joint browser evidence, including all animation phases and reserved domains','Materialize a private index file, generate derivative payloads only in private candidate directories, and verify them','Reconcile source ownership and current catalog/map; preserve intervening unrelated edits or stop'],ordering=['Back up every existing destination byte-for-byte with hash manifest; record originally absent paths','Copy complete candidate asset/resource payloads and verified derivatives','Atomically replace the rebased map and rebuilt index last','Revalidate strict loader and live editor runtime'],rollback=['Restore exact previous map/index and each replaced payload from hash-checked backups','Remove only newly introduced paths listed in that publication transaction','Keep review and approval evidence'],browser_request_sha256=sha(B/'browser-request.json'))
 write(B/'publication-plan.json',plan);print(json.dumps(dict(pins=len(pins),staged_files=len(writes),browser_request_sha256=sha(B/'browser-request.json'))))
if __name__=='__main__':main()
