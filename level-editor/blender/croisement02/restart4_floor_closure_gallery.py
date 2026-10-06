"""Freeze the root-reviewed final non-relief floor input card."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart4-floor-closure-input-v1';p=json.loads((d/'proposal.json').read_text())
 assert sha(d/'proposal.json')=='40bdfaec7930ae02a60127e5de8e1bf6a64ba783be9a698db28f5c66315c93b8'
 receipt=dict(status='ready-for-user',scope='5917 inferred underlying floor +13 exact native returns input reuse only; not foreground completion',proposal_sha256=sha(d/'proposal.json'),reviewer='root',evidence='Root personally viewed all five source/current/reuse sheets and native13 close and passed exact input scope. No synthesis or bake approval inferred.',user_approval=None)
 write_json(d/'root-review.json',receipt)
 item=dict(id='croisement02-final-nonrelief-floor-input',name='Remaining flat floor — finite5930-pixel reuse input',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='source-region and shown existing-fill reuse input only',model=p['base_model'],solid=str(d/'source-reuse-contexts-0.png'),solid_label='Original source / coverage / approved floor / proposed reuse — major contexts',textured=str(d/'source-reuse-contexts-1.png'),textured_label='Remaining source contexts2/5',source_comparison=str(d/'source-reuse-contexts-2.png'),source_comparison_label='Source contexts3/5, including phase-owned pixels',source_comparison_secondary=str(d/'source-reuse-contexts-3.png'),source_comparison_secondary_label='Source contexts4/5',source_trace=str(d/'source-reuse-contexts-4.png'),source_trace_label='Source contexts5/5',projection_errors=str(d/'native13-close.png'),projection_errors_label='All13 exact native background returns',validation=str(d/'validation.json'),review=str(d/'root-review.json'),notes=[
 'Approve only the exact displayed5917 inferred underlying-floor pixels and13 exact native returns using the existing raw response. No new synthesis requested. Saved-model appearance will be reviewed separately.',
 'Whole-scene native first hits classify487 centers as exposed floor and5443 as covered by static assets. Pink source-overlay pixels are exposed; orange are covered. Some exposed pixels coincide with missing foreground wood/silhouette coverage, which remains a separate geometry obligation.',
 'All772238 known pixels and every pixel outside5930 remain exact; no geometry, UV, alpha, bank appearance, state artwork or source ownership changes.',
 'Elevated hiding-Pc initial/terminal effects and animated Arbre03 remain separate; their artwork is never painted onto floor. Thirteen background returns have no initial/terminal/ambient frame0 alpha and match both frozen background sources.',
 'Existing reused texture can be softer and retain inferred shadows. Legacy569880 bank-underlay exclusion and the separately diagnosed rear bank-edge floor patch are excluded. This card does not claim whole-scene or foreground completion.'
 ])
 write_json(d/'review-candidates.json',dict(map='Croisement02 remaining flat-floor input',items=[item],without_packets=[],status_counts={'pending input approval':1}));build(d/'review-candidates.json',d/'gallery');write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',proposal_sha256=sha(d/'proposal.json'),candidates=str(d/'review-candidates.json'),gallery=str(d/'gallery/index.html'),scope=item['review_scope'],user_approval=None))
if __name__=='__main__':main()
