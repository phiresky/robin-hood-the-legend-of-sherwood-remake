"""Freeze the independently reviewed northwest rock contact correction."""
import json
from pathlib import Path
import sys
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
    folder=OUT/'restart2-bank321/northwest-edge-ramp-v1'
    gallery=folder/'geometry-gallery'
    if gallery.exists():raise FileExistsError(gallery)
    model=sha(folder/'worker.blend')
    root=json.loads((folder/'root-review.json').read_text())
    preservation=json.loads((folder/'reopened-review/preservation.json').read_text())
    support=json.loads((folder/'underside-support.json').read_text())
    if root['status']!='ready-for-user-new-geometry-review' or root['model_sha256']!=model:raise ValueError('Exact root readiness required')
    if preservation['status']!='PASS' or preservation['model_sha256']!=model:raise ValueError('Exact reopened preservation required')
    if support['model_sha256']!=model or support['samples_without_bank'] or support['positive_gap_samples']:raise ValueError('Unsupported rock underside')
    summary={k:v for k,v in support.items() if k!='rows'}
    summary.update(status='PASS: sampled underside remains embedded in approved bank',full_report_sha256=sha(folder/'underside-support.json'))
    write_json(folder/'support-summary.json',summary)
    notes=[
        'New geometry decision: only the right edge of rear rock part 035 moves along the original camera ray, ramping from no movement at native X=80 to full movement at X=105. All other objects, UVs, material data and packed images remain exact. Existing approvals do not approve this revision.',
        'The correction recovers 307 native rock pixels previously covered by the bank, including all 167 physically recoverable pixels in the diagnosed gap. Every vertex retains its original source-camera projection within 0.000031 pixels.',
        '95 additional mixed boundary pixels become rock-first instead of bank-first. Their native source ownership remains unchanged; this small ambiguous edge is disclosed, not reassigned to rock.',
        '66,117 underside samples all hit the approved bank, with no floating gap. The sampled underside is buried 26.816–43.949 world units. Gray underside faces visible in isolated side views remain below the physical bank surface.',
        '119 native rock-domain pixels remain outside current first-hit coverage: 45 silhouette misses and 74 behind terrain. This scoped correction does not claim complete native coverage.',
        'The earlier whole-rock movement was rejected because it exposed a gray underside over the lower rock cap. This bounded correction preserves that cap. First view in both eight-view sheets is the original game camera.',
    ]
    item=dict(id='croisement02-northwest-rock-outcrop',name='Northwest rock — bounded bank-contact correction',status='ready-for-user',technical_eligible=True,user_approval=None,
        model=str(folder/'worker.blend'),
        solid=str(OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/bake-single-v2/actual/textured.png'),solid_label='Previous geometry with existing texture — original game camera first',
        textured=str(folder/'reopened-review/actual8.png'),textured_label='Proposed geometry with exact same texture — original game camera first',
        source_comparison=str(folder/'source-baseline-candidate.png'),source_comparison_label='Native artwork / previous bank contact / corrected bank contact',
        source_trace=str(OUT/'restart2-bank321/northwest-boundary-v1/marked.png'),source_trace_label='95 ambiguous border pixels highlighted in magenta; ownership unchanged',
        source_comparison_secondary=str(folder/'oblique-contact-0.png'),source_comparison_secondary_label='Physical bank contact — oblique 1',
        projection_errors=str(folder/'oblique-contact-1.png'),projection_errors_label='Physical bank contact — oblique 2',
        validation=str(folder/'reopened-review/preservation.json'),ownership=str(folder/'support-summary.json'),review=str(folder/'root-review.json'),notes=notes)
    index=folder/'geometry-review-candidates.json'
    write_json(index,dict(map='Croisement02 northwest rock contact',items=[item],without_packets=[],status_counts={'pending geometry review':1}))
    build(index,gallery)


if __name__=='__main__':main()
