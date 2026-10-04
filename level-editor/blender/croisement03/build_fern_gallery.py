"""Publish only fern workers with a current, explicit saved-image self-review."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from build_review_gallery import build


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('round_directory', type=Path)
    args = parser.parse_args()
    directory = args.round_directory.resolve(strict=True)
    items = []
    for worker in sorted((directory / 'assets').iterdir()):
        review_path = worker / 'inspection/visual-review.json'
        review = json.loads(review_path.read_text())
        if review['status'] != 'ready-for-geometry-review':
            raise ValueError(f'Worker has no ready self-review: {worker.name}')
        for name, expected in review['evidence'].items():
            if hashlib.sha256((worker / name).read_bytes()).hexdigest() != expected:
                raise ValueError(f'Stale visual review: {worker.name}/{name}')
        if json.loads((worker / 'validation.json').read_text())['status'] != 'PASS':
            raise ValueError('Worker validation failed')
        coverage = review['native_coverage']
        if coverage['missing_pixels']:
            raise ValueError('Unresolved missing native pixels')
        ownership = sorted((worker / 'projection').glob('*/ownership.json'))
        ownership = [p for p in ownership if p.parent.name != 'input']
        if len(ownership) != 1:
            raise ValueError('Ambiguous projection ownership')
        items.append(dict(
            id=worker.name, name='North Woodland Fern ' + worker.name.rsplit('-', 1)[-1],
            status='ready-for-user', technical_eligible=True, model=str(worker/'model.blend'),
            solid=str(worker/'modified/solid.png'), textured=str(worker/'modified/textured.png'),
            context=str(worker/'modified/context.png'),
            stored_material_textured=str(worker/'inspection/actual-materials/sheet.png'),
            source_comparison=str(worker/'inspection/source-comparison/comparison.png'),
            source_comparison_label='Exact native camera: source, saved geometry, overlay and edge differences',
            source_comparison_secondary=str(worker/'inspection/ground-contact/sheet.png'),
            source_comparison_secondary_label='Ground contact with unchanged coarse neighboring wood — all eight views',
            validation=str(worker/'validation.json'), ownership=str(ownership[0]),
            stored_material_audit=str(worker/'inspection/saved-source-audit.json'),
            review=str(review_path),
            notes=[f"Geometry candidate; native mask {coverage['native_mask']}, {coverage['missing_pixels']} missing / {coverage['extra_pixels']} edge pixels."] + review['limitations'],
        ))
    manifest = directory/'review-candidates.json'
    manifest.write_text(json.dumps(dict(map='Croisement03 Ferns', items=items,
        status_counts={'ready for geometry review':len(items), 'user approved':0}), indent=2)+'\n')
    build(manifest, directory/'gallery', pending_only=True)
    print(directory/'gallery/index.html')


if __name__ == '__main__':
    main()
