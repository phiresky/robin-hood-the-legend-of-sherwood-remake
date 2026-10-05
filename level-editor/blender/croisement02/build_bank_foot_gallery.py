"""Expose a new bank-foot geometry decision while preserving previous approval."""
import json
from pathlib import Path
import sys
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
    worker=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank'
    inspection=worker/'inspection';root=json.loads((inspection/'foot-root-review.json').read_text())
    model=sha(worker/'model.blend')
    if root['model_sha256']!=model or root['status']!='ready-for-user-new-geometry-review':raise ValueError('Exact root-reviewed bank required')
    audit=json.loads((inspection/'saved-model-audit.json').read_text())
    if audit['status']!='PASS' or audit['model_sha256']!=model:raise ValueError('Exact saved audit required')
    reviewed=['actual-materials/sheet.png','baseline-comparison/comparison-0-3.png','baseline-comparison/comparison-4-7.png','foot-native-compare-0.png','foot-native-compare-1.png']+[f'foot-contact-review/contact-{i}-actual.png' for i in range(3)]
    notes=['New geometry decision: 11 low vertices of bank part 000 move outward at most 5.596 native pixels, closing 32 true toe gaps plus 70 adjacent source pixels. Heights, crest, unrelated geometry, the prior fill model’s UVs and packed image data are unchanged.',
           'All changed foot vertices still meet the exact approved flat ground receiver at Z=0. There are no newly added slabs or enlarged plateau surfaces.',
           '217 narrow historical source-edge misses remain disclosed. The separate whole-scene material/source-role audit is not closed by this foot correction.',
           'Actual-material views retain the existing generated bank texture for inspection. This geometry approval does not inherit or grant texture approval. Original approved bank 5fefeb remains archived unchanged.',
           'Source-projection views use native pixels and gray unknowns. Ground contact views deliberately show remaining unknown floor gray.']
    write_json(inspection/'visual-review.json',dict(model_sha256=model,ready_for_geometry_review=True,status='ready-for-user-new-geometry-review',root_review_sha256=sha(inspection/'foot-root-review.json'),reviewed_images={n:sha(inspection/n) for n in reviewed},findings=notes,user_approval=None,prior_texture_approval_inherited=False))
    item=dict(id='croisement02-north-woodland-bank',name='North woodland bank — small foot-contour correction',status='ready-for-user',technical_eligible=True,user_approval=None,
        model=str(worker/'model.blend'),solid=str(worker/'modified/solid.png'),textured=str(worker/'modified/textured.png'),context=str(worker/'modified/context.png'),
        stored_material_textured=str(inspection/'actual-materials/sheet.png'),
        source_comparison=str(inspection/'foot-native-compare-0.png'),source_comparison_label='Western toe: native artwork and corrected actual material',
        source_comparison_secondary=str(inspection/'foot-native-compare-1.png'),source_comparison_secondary_label='Northeast toe: native artwork and corrected actual material',
        projection_errors=str(inspection/'foot-contact-review/contact-0-actual.png'),projection_errors_label='Native-camera contact with approved ground receiver',
        artwork_references=[dict(id=f'contact-{i}',label=f'Oblique ground contact {i}',path=str(inspection/f'foot-contact-review/contact-{i}-actual.png'),sha256=sha(inspection/f'foot-contact-review/contact-{i}-actual.png')) for i in [1,2]],
        validation=str(worker/'validation.json'),stored_material_audit=str(inspection/'saved-model-audit.json'),ownership=str(inspection/'foot-validation.json'),review=str(inspection/'visual-review.json'),notes=notes)
    index=worker/'review-candidates.json';write_json(index,dict(map='Croisement02 bank foot correction',items=[item],without_packets=[],status_counts={'pending geometry review':1}))
    build(index,worker/'gallery')


if __name__=='__main__':main()
