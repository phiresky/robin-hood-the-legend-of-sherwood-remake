"""Bind independently reviewed bridge geometry to its pending-user gallery."""
import hashlib
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from build_review_gallery import build
OUT = ROOT / 'level-editor/work/croisement03-refinement'
EXPECTED = '017c016a13ebfe108b5402d71ad9541f29515e1f0d22486521665015b79ab380'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    directory = OUT / 'restart2/bridge-v6'
    worker = directory / 'assets/croisement03-timber-bridge'
    joint = OUT / 'restart2/bridge-contact-v2'
    assert sha(worker / 'model.blend') == EXPECTED
    assert json.loads((worker / 'validation.json').read_text())['status'] == 'PASS'
    coverage = json.loads((worker / 'inspection/native-rail-coverage.json').read_text())
    assert coverage['model_sha256'] == EXPECTED
    assert sum(r['expected'] for r in coverage['native_rails']) == 3434
    assert sum(r['missing'] for r in coverage['native_rails']) == 0
    contact = json.loads((joint / 'evidence.json').read_text())
    assert contact['model_sha256'] == EXPECTED
    assert contact['sheet_sha256'] == sha(joint / 'sheet.png')
    limitations = [
        'Geometry review only; no user approval or generated texture fill yet.',
        'All 3434 native rail and pier mask pixels are covered. Authored deck/fascia has 166 missing boundary pixels, at most 2 source pixels deep.',
        'Deck height follows the native zero-elevation frame. Deck thickness, water depth and riverbed are inferred.',
        'Gray rear and hidden faces have no observed source texture; texture fill remains pending.',
        'Joint views use explicit provisional bank solids and water. Full-map banks, shore rocks and ground replacement remain unfinished.',
        'Inspection sunlight is provisional; final map lighting calibration remains pending.',
    ]
    evidence = {str(p.relative_to(OUT)): sha(p) for p in [
        worker/'model.blend', worker/'validation.json',
        worker/'inspection/actual-materials/sheet.png',
        worker/'inspection/source-comparison/comparison.png',
        worker/'inspection/native-rail-coverage.json', joint/'joint.blend',
        joint/'sheet.png', joint/'evidence.json']}
    review = worker / 'inspection/visual-review.json'
    review.write_text(json.dumps(dict(status='ready-for-geometry-review',
        model_sha256=EXPECTED, user_approved=False,
        self_review='PASS: inspected saved actual materials in eight views, native source overlay and explicit land/water/bed contact.',
        independent_review='Root independently inspected actual eight views, source comparison and joint contact eight views. Scoped geometry PASS: thin rail/bracket profiles, northwest gap, deck proportions and central support credible.',
        native_coverage=coverage, evidence=evidence, limitations=limitations), indent=2)+'\n')
    ownership = [p for p in (worker/'projection').glob('*/ownership.json') if p.parent.name != 'input']
    assert len(ownership) == 1
    item = dict(id=worker.name, name='Timber bridge', status='ready-for-user',
        technical_eligible=True, model=str(worker/'model.blend'),
        solid=str(worker/'modified/solid.png'), textured=str(worker/'modified/textured.png'),
        context=str(worker/'modified/context.png'),
        stored_material_textured=str(worker/'inspection/actual-materials/sheet.png'),
        source_comparison=str(worker/'inspection/source-comparison/comparison.png'),
        source_comparison_label='Native source and saved geometry: inferred deck, exact rail/pier coverage',
        source_comparison_secondary=str(joint/'sheet.png'),
        source_comparison_secondary_label='Actual bridge against provisional bank solids, water and seated riverbed',
        validation=str(worker/'validation.json'), ownership=str(ownership[0]),
        review=str(review), notes=limitations)
    manifest = directory/'review-candidates.json'
    manifest.write_text(json.dumps(dict(map='Croisement03 timber bridge', items=[item],
        status_counts={'ready for geometry review':1,'user approved':0}), indent=2)+'\n')
    build(manifest,directory/'gallery',pending_only=True)
    print(directory/'gallery/index.html')
if __name__ == '__main__': main()
