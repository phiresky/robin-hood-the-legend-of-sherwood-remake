"""Freeze reviewed inferred bank-underlay input without changing bank authority."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart4-bank-underlay-input-v1';p=json.loads((d/'proposal.json').read_text());assert sha(d/'proposal.json')=='b639342dfa2b2f8ba8750f80c2595e35385dd4d50fbb9519ceceb323915708fe'
 write_json(d/'root-review.json',dict(status='ready-for-user',reviewer='root',proposal_sha256=sha(d/'proposal.json'),scope='569880 inferred underlying flat-floor pixels using shown existing raw response; bank source/material/geometry unchanged',evidence='Root personally viewed physical context, whole source guide and all eight regional comparisons; scoped input PASS. No claim all domain pixels are visible.',user_approval=None))
 item=dict(id='croisement02-bank-underlay-floor-input',name='Bank underlay — remaining inferred flat-floor texture input',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='exact inferred underfloor region and shown existing-fill reuse input only',model=p['base_model'],solid=str(d/'physical-context.png'),solid_label='Original camera first; reverse visible gray floor and separately covered field gap',textured=str(d/'source-region-guide.png'),textured_label='Orange: entire finite legacy bank-underlay domain',source_comparison=str(d/'full-domain-regions-0.png'),source_comparison_label='Northern regions: source / domain / approved floor / proposed reuse',source_comparison_secondary=str(d/'full-domain-regions-1.png'),source_comparison_secondary_label='Remaining regions: source / domain / approved floor / proposed reuse',source_trace=str(d/'proposed-appearance.png'),source_trace_label='Exact proposed flat-ground atlas; bank itself is unchanged',projection_errors=str(OUT/'restart2-ground-completion/preparation-v1/reference-review.png'),projection_errors_label='Four native floor examples supporting the existing raw material',validation=str(d/'validation.json'),review=str(d/'root-review.json'),notes=[
 'Approve569880 inferred underlying flat-floor pixels using the shown existing raw response. Zero native returns; no new synthesis requested. Saved-model appearance remains a later review.',
 'This domain was excluded by an older bank-versus-ground native first-hit test. The current scene physically exposes84 sampled texels in the reverse gray triangle; all remaining domain pixels are conservatively inferred underlay, not claimed visible.',
 'Bank source artwork, geometry, materials, height and coverage stay untouched. This proposal does not repaint bank surfaces or assign foreground source art to ground.',
 'All772238 known floor pixels and all outside pixels remain exact. The separately proposed5930-pixel floor scope is disjoint; a future approved composite must preserve both deltas.',
 'Existing reused forest floor is softer and includes inferred shadows. Closing neutral flat-ground atlas gaps is not proof of complete foreground geometry or arbitrary-view bank coverage.'
 ])
 write_json(d/'review-candidates.json',dict(map='Croisement02 bank-underlay input',items=[item],without_packets=[],status_counts={'pending input approval':1}));build(d/'review-candidates.json',d/'gallery');write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',proposal_sha256=sha(d/'proposal.json'),candidates=str(d/'review-candidates.json'),gallery=str(d/'gallery/index.html'),scope=item['review_scope'],user_approval=None))
if __name__=='__main__':main()
