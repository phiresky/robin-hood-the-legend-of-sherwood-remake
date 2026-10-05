"""Freeze one final review of the source-traced northwest ledge correction."""
import json
from pathlib import Path
import sys
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
    folder=OUT/'restart2-bank321/northwest-ledge26-v2';gallery=folder/'geometry-gallery'
    if gallery.exists():raise FileExistsError(gallery)
    model=sha(folder/'worker.blend');root=json.loads((folder/'root-review.json').read_text())
    validation=json.loads((folder/'reopened-review/preservation.json').read_text());coverage=json.loads((folder/'full-source-audit.json').read_text())
    if root['status']!='ready-for-user-new-geometry-review' or root['model_sha256']!=model:raise ValueError('Exact root readiness required')
    if validation['status']!='PASS' or validation['model_sha256']!=model or validation['maximum_supported_base_change']!=0:raise ValueError('Preservation or support guard failed')
    if coverage['model_sha256']!=model or coverage['new_foreign_pixels'] or coverage['regressed_pixels']:raise ValueError('Source footprint guard failed')
    if any(r['classification']!='partially covered terrain-contact edge' for r in coverage['edge_rows']):raise ValueError('Unresolved ledge edge')
    baseline=OUT/'restart2-bank321/northwest-edge-ramp-v1'
    item=dict(id='croisement02-northwest-rock-outcrop',name='Northwest rock — small native ledge completion',status='ready-for-user',technical_eligible=True,user_approval=None,
        model=str(folder/'worker.blend'),
        solid=str(baseline/'reopened-review/actual8.png'),solid_label='Approved geometry — original game camera first',
        textured=str(folder/'reopened-review/actual8.png'),textured_label='Proposed small ledge completion — original game camera first',
        source_comparison=str(folder/'source-before-after-labeled.png'),source_comparison_label='Native artwork / approved model / small ledge completion',
        source_comparison_secondary=str(folder/'oblique-contact-0.png'),source_comparison_secondary_label='Physical bank contact — oblique 1',
        projection_errors=str(folder/'oblique-contact-1.png'),projection_errors_label='Physical bank contact — oblique 2',
        validation=str(folder/'reopened-review/preservation.json'),ownership=str(folder/'full-source-audit.json'),review=str(folder/'root-review.json'),
        notes=[
            'The small right ledge extends laterally by at most 4.775 native pixels to match the original artwork. Existing topology, heights, depth, other objects and the supported rock base are unchanged.',
            'Existing UV maps, materials and packed images remain exact. A local layer of the original artwork colors the corrected front faces; no new generated texture was requested.',
            '23 previously hidden native pixel centers become visible, with no new foreign-source occlusion or coverage regression. The three remaining edge pixels already contain visible rock over 26–45% of their area; their centers lie less than 0.283 pixels from the slanted terrain-contact boundary.',
            '71 moss/ground-fringe pixels and 22 older narrow-edge ambiguities remain documented without ownership changes. This review concerns the small ledge, not automatic reassignment of those pixels.',
            'The previously approved a04549e5 model remains archived unchanged. This new geometry revision requires its own decision.',
        ])
    index=folder/'geometry-review-candidates.json'
    write_json(index,dict(map='Croisement02 northwest rock ledge',items=[item],without_packets=[],status_counts={'pending geometry review':1}))
    build(index,gallery)


if __name__=='__main__':main()
