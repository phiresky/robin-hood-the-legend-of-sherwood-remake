"""Freeze a geometry-only market well card for the next grouped review."""
import sys,json,hashlib,os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement/blender')]
from build_review_gallery import build
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-market-well-v4';F=B/'restart7-market-well-frozen-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 digest=sha(D/'model.blend');audit=json.loads((D/'review.json').read_text());assert audit['model_sha256']==digest
 assert (D/'root-review.json').exists(), 'Coordinator review required before freeze'
 receipt=json.loads((D/'root-review.json').read_text());assert receipt['model_sha256']==digest and receipt['status'].startswith('PASS');F.mkdir(exist_ok=False);os.link(D/'model.blend',F/'model.blend');write(F/'root-review.json',receipt)
 item=dict(id='york-market-roofed-stone-well',name='York market roofed stone well',status='ready-for-user',technical_eligible=True,model=str(F/'model.blend'),solid=str(D/'review/solid-eight.png'),textured=str(D/'review/actual-eight.png'),context=str(D/'review/source-comparison.png'),source_comparison=str(D/'review/source-comparison.png'),source_comparison_label='Native artwork and saved well/pail geometry on exact raised street',validation=str(D/'review.json'),ownership=str(D/'source-guard.json'),review=str(F/'root-review.json'),notes=f"Closed hollow stone ring, timber roof posts and support beam, two pitched roof shells, and a small open tapered pail. Native mask49/50 and the separately traced pail domain retain original RGB. Saved-model native first hits: {audit['own_first_hit']}/{audit['mask_domain_pixels']}; {len(audit['foreign_first_hits'])} floor boundary centers, {len(audit['outside_domain_first_hits'])} outside-domain centers, all explicitly disclosed. Unseen surfaces remain gray; grazing roof projection has low-resolution stretching. Raised terrain086 is unchanged context.",approval_scope='Exact well and pail geometry only. Pail source role/domain are explicitly inferred for inspection; unseen texture, gameplay and publication remain unapproved.',animation_reviews=[dict(id='well-context',name='Well, pail and raised street contacts',description='Unmodified neighbor/terrain in four physical views; native camera first.',textured=str(D/'review/contact-four.png'),solid=str(D/'review/solid-eight.png'),context=str(D/'review/source-comparison.png'))])
 write(F/'review-candidates.json',dict(map='York',review_kind='geometry',scope='Next grouped batch; no individual question.',items=[item]));build(F/'review-candidates.json',F/'gallery');write(F/'freeze-receipt.json',dict(status='Frozen next-pool geometry card',model_sha256=digest,files={str(p.relative_to(F)):sha(p)for p in F.rglob('*')if p.is_file()},user_approved=False,publication=False));print(F/'gallery/index.html')
if __name__=='__main__':main()
