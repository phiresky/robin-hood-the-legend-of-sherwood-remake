"""Freeze a geometry-only riverside storehouse card for the next grouped review."""
import sys,json,hashlib,os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement/blender')]
from build_review_gallery import build
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-riverside-storehouse-v2';F=B/'restart7-riverside-storehouse-frozen-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 digest='49bf6581b3d1e90b246d8994048932041e8f0e3fed94ccd965aede22930cc4ed';assert sha(D/'model.blend')==digest
 assert (D/'root-review.json').exists(), 'Coordinator review required before freeze'
 receipt=json.loads((D/'root-review.json').read_text());assert receipt['model_sha256']==digest and receipt['status'].startswith('PASS');F.mkdir(exist_ok=False);os.link(D/'model.blend',F/'model.blend');write(F/'root-review.json',receipt)
 item=dict(id='york-riverside-stone-storehouse',name='York riverside stone storehouse',status='ready-for-user',technical_eligible=True,model=str(F/'model.blend'),solid=str(D/'review/solid-eight.png'),textured=str(D/'review/actual-eight.png'),context=str(D/'review/source-comparison.png'),source_comparison=str(D/'review/source-comparison.png'),source_comparison_label='Native artwork and saved geometry in frozen shed/street context',validation=str(D/'review.json'),ownership=str(D/'source-guard.json'),review=str(F/'root-review.json'),notes='Closed masonry with seven shallow window/slit/arched-door recesses, four closed hip-roof shells and closed chimney. Native mask0 has 35,441 own first hits, 2,942 delegated to the exact frozen shed, four floor hits and 94 remaining boundary misses. Original RGB and accepted alpha are exact; source on hidden neighboring receivers is excluded. Backs and the shed-hidden wall remain gray. The retained hidden floor plane is inferred, with 0.000354 scene-unit clearance; front terrain is 0.121 lower. Shed and terrain are unchanged context.',approval_scope='Exact storehouse geometry only, with source-facing appearance for inspection. No generated texture, new source ownership, state or publication decision.',animation_reviews=[dict(id='storehouse-context',name='Frozen shed and raised street contacts',description='Unmodified neighbor/terrain in four physical views; native camera first.',textured=str(D/'review/contact-four.png'),solid=str(D/'review/solid-eight.png'),context=str(D/'review/source-comparison.png'))])
 write(F/'review-candidates.json',dict(map='York',review_kind='geometry',scope='Next grouped batch; no individual question.',items=[item]));build(F/'review-candidates.json',F/'gallery');write(F/'freeze-receipt.json',dict(status='Frozen next-pool geometry card',model_sha256=digest,files={str(p.relative_to(F)):sha(p)for p in F.rglob('*')if p.is_file()},user_approved=False,publication=False));print(F/'gallery/index.html')
if __name__=='__main__':main()
