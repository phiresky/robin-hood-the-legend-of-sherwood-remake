"""Package the reviewed finite floor reuse and exact background-return input proposal."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart3-remaining-floor-input-v1';p=json.loads((d/'proposal.json').read_text());r=json.loads((d/'root-review.json').read_text());assert r['status']=='ready-for-user' and r['proposal_sha256']==sha(d/'proposal.json')
 item=dict(id='croisement02-remaining-floor-reuse-input',name='Remaining context floor — inferred reuse and49 native returns',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='source-region and existing-fill reuse input only',model=p['base_model'],solid=str(d/'region-guide.png'),solid_label='Orange:14,876 inferred pixels; cyan:49 exact native returns',textured=str(d/'proposed-appearance.png'),textured_label='Exact proposed bounded composite; no model change',source_comparison=str(d/'major-reuse-crops.png'),source_comparison_label='Major source contexts: original/source domain, saved base, proposed reuse',source_comparison_secondary=str(d/'remaining-reuse-crops.png'),source_comparison_secondary_label='Remaining source contexts and bounded reuse',source_trace=str(d/'native49-comparison.png'),source_trace_label='49 transient-reservation pixels: current / original / exact native return',projection_errors=str(OUT/'restart3-remaining-floor-audit-v1/classified-views.png'),projection_errors_label='Original-camera and oblique gray-floor diagnosis; blue/purple already handled separately',validation=str(d/'validation.json'),review=str(d/'root-review.json'),notes=[
 'Input approval only: authorize14,876 inferred floor pixels from the exact shown existing raw response plus49 exact native background returns. No new API request is proposed. Saved-model appearance review follows separately.',
 'The49 native pixels were excluded only by temporary hole-effect frames1–8 in two identical mission replicas. Neither initial nor final artwork covers them. Their original RGBA is restored exactly; the dynamic overlay remains intact.',
 'The returned native pixels are sharper/brighter than nearby inferred floor. Reused grass/soil/shadow is softer/coarser than original art. These differences are disclosed in the close comparisons.',
 'All772,189 known pixels, prior5,419/2,441/8,201/14-pixel changes, bank relief, alpha and all other pixels remain exact. Foreground artwork retains its existing owners; no wall, tree or prop source is copied onto floor.',
 'Depends on separate approval of the868 cumulative ground appearance card. This is the finite59-component set from six inspected state-context views; it is not a whole-map completion claim.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 remaining context floor input',items=[item],without_packets=[],status_counts={'pending source region and material reuse input':1}));build(index,d/'gallery');page=d/'gallery/index.html';page.write_text(page.read_text().replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Exact source-region and existing-material input proposal. Saved-model appearance approval remains separate.'));write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope='source-region and existing-material input only',proposal_sha256=sha(d/'proposal.json'),candidates=str(index),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
