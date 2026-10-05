"""Freeze a reviewed under-canopy floor-domain and existing-material input proposal."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart3-initial-fence/tree45-floor-proposal-v1';p=json.loads((d/'proposal.json').read_text());r=json.loads((d/'root-review.json').read_text());assert r['status']=='ready-for-user' and r['proposal_sha256']==sha(d/'proposal.json')
 context=OUT/'restart3-initial-fence/tree45-reservation-context-v1'
 item=dict(id='croisement02-tree45-under-canopy-floor-input',name='Tree45 hidden floor — inferred region and existing fill reuse',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='source-region and existing-fill input approval only',model=p['ground_model'],solid=str(d/'source-role-close.png'),solid_label='Native foliage stays tree-owned; orange is underlying inferred floor',textured=str(d/'reuse-close.png'),textured_label='Current floor / existing raw response / exact bounded reuse preview',source_comparison=str(context/'source-context.png'),source_comparison_label='Original camera: source / tree omitted / approved tree45 context',source_comparison_secondary=str(context/'oblique.png'),source_comparison_secondary_label='Reverse view reveals unknown floor beneath the canopy',source_trace=p['reference_sheet'],source_trace_label='Four exact native ground references used for the existing response',projection_errors=str(d/'region-guide.png'),projection_errors_label='Orange only:2,441 editable pixels; every other pixel protected',validation=str(d/'validation.json'),review=str(d/'root-review.json'),notes=[
 'Input approval only: authorize guarded reuse of the exact existing generated grass/soil in2,441 inferred floor pixels. No new synthesis is requested; baked appearance remains separately reviewable.',
 'All2,441 native source pixels are physically covered by approved tree45 in the original camera. Reverse views expose the underlying floor. Native foliage remains tree-owned and is never copied onto ground.',
 'All772,189 known ground pixels, the previous5,419-pixel fence-floor candidate and the approved14-pixel trap correction remain unchanged. This proposal is disjoint from the separately proposed8,201-pixel state underlay.',
 'The proposed existing fill is softer than native ground. The preview applies only the new mask; it does not accept the raw response elsewhere. No model has been changed.',
 'The base4e2c98fb floor appearance is a separate pending card in this review pool. Ground geometry and UVs are unchanged, and terminal cleared-fence artwork remains applied-only.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 tree45 hidden floor input',items=[item],without_packets=[],status_counts={'pending inferred-floor input':1}));build(index,d/'gallery')
 page=d/'gallery/index.html';page.write_text(page.read_text().replace('Croisement02 tree45 hidden floor input model review','Croisement02 tree45 hidden floor input review').replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Inferred floor domain and existing-material reuse proposal. No model mutation or new synthesis.'))
 write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope='inferred floor domain and existing-fill input only',proposal_sha256=sha(d/'proposal.json'),candidates=str(index),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
