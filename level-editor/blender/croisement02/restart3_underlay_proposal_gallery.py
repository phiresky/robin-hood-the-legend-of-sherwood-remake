"""Freeze a bounded floor-domain and shown material-reuse proposal."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart2-state/underlay-input-review-v1';p=json.loads((d/'proposal.json').read_text());r=json.loads((d/'root-review.json').read_text());assert r['status']=='ready-for-user' and r['proposal_sha256']==sha(d/'proposal.json')
 context=OUT/'restart2-state/underlay-context-proposal-v1';revealed=OUT/'restart2-state/underlay-rock-revealed-diagnostic-v1';audit=OUT/'restart2-state/underlay-aggregate-audit-v3'
 item=dict(id='croisement02-trap-cart-underlay-input',name='Trap and cart floor —8,201 bounded inferred pixels',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='inferred floor domain and shown existing-fill reuse',model=str(Path(p['receiver_model']).resolve()),solid=str(audit/'source-domain-comparison.png'),solid_label='Exact four floor domains: source / current / existing response / proposed reuse',textured=str(context/'labeled-comparison.png'),textured_label='Original camera first: current and proposed floor in unchanged approved model context',source_comparison=str(revealed/'labeled-diagnostic.png'),source_comparison_label='Revealed rock diagnostic: vegetation hidden, ground and rocks unchanged',source_comparison_secondary=str(revealed/'visible-floor-close.png'),source_comparison_secondary_label='Magnified native and oblique floor gap before/proposed',projection_errors=str(d/'domain-overlay.png'),projection_errors_label='Magenta only:8,201 editable pixels; all other source pixels protected',validation=str(d/'proposal.json'),review=str(d/'root-review.json'),notes=[
 'Approve only the exact inferred-floor domain and shown reuse of the existing generated floor. No new API request or generated response. Equivalent guarded saved-model application remains to be verified.',
 'Four disjoint source-bound regions: logs6,970; rocks81; south cart184; north cart966. All are already classified as ground-owned deferred floor.',
 'Every known source pixel, bank relief, the complete initial-fence rectangle, and approved14-pixel continuation stay unchanged. This domain is disjoint from separate5,419 and2,441 floor proposals.',
 'Larger gray patches outside these regions remain incomplete and unchanged. This is not complete cart/terrain repair or whole-map approval.',
 'Approved endpoint geometry and textures are unchanged. Dynamic native artwork, shadows and applied fence ground remain independently activated state resources.',
 'Upper-left view is the original game camera. The rock diagnostic hides vegetation explicitly to expose the tiny ground gap; ordinary context images retain the selected approved vegetation.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 trap and cart floor proposal',items=[item],without_packets=[],status_counts={'pending inferred-floor domain and reuse':1}));build(index,d/'gallery')
 page=d/'gallery/index.html';page.write_text(page.read_text().replace('Croisement02 trap and cart floor proposal model review','Croisement02 trap and cart floor proposal review').replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Inferred floor domain and shown existing-material reuse. No new synthesis or canonical model change.'))
 write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope=item['review_scope'],proposal_sha256=sha(d/'proposal.json'),candidates=str(index),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
