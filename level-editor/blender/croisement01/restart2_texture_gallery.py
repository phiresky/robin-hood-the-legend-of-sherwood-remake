"""Freeze independently reviewed tree fills for explicit user texture review."""
import argparse
import json
import shutil
import sys
from pathlib import Path
from restart2_ready_gallery import OUT, ROOT, sha
from restart2_review_labels import labeled_sheet
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from build_review_gallery import build

CASES = {
    '18': ('tree18-v4', 'experiment-v3', 'baked-v1', 'Northeast Forked Forest Tree'),
    '20': ('tree20-v3', 'experiment', 'baked-v2-luminance', 'Northern Shaded Forest Tree'),
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('destination')
    parser.add_argument('trees', nargs='+', choices=CASES)
    args = parser.parse_args()
    dest = OUT / args.destination
    dest.mkdir(exist_ok=False)
    (dest / 'models').mkdir()
    (dest / 'receipts').mkdir()
    items, archive = [], []
    for number in args.trees:
        original, experiment, bake, name = CASES[number]
        asset = 'croisement01-tree-' + number
        root = OUT / 'approved-tree-fills-v1' / asset
        e, b = root / experiment, root / bake
        w = b / 'actual-review-v1'
        geometry = OUT / original / 'assets' / asset
        approved = root / 'approved-workspace'
        for relative in ('modified/solid.png', 'modified/textured.png', 'modified/views.json'):
            assert sha(approved / relative) == sha(geometry / relative)
        proof = json.loads((b / 'reopened-preservation.json').read_text())
        receipt = json.loads((w / 'inspection/root-review.json').read_text())
        model_hash = sha(w / 'model.blend')
        assert model_hash == sha(b / 'worker.blend') == proof['candidate_model_sha256'] == receipt['model_sha256']
        assert proof['reopened_preservation'] == 'PASS'
        assert receipt['status'] == 'scoped texture appearance PASS'
        for key, value in receipt['files'].items():
            assert sha(w / key) == value
        for key in ('geometry_unchanged', 'foreign_appearance_unchanged', 'physical_alpha_unchanged',
                    'known_foliage_rgba_unchanged', 'foliage_uv_and_ownership_unchanged'):
            assert proof[key] is True
        frozen = dest / 'models' / f'{asset}-{model_hash[:16]}.blend'
        shutil.copy2(w / 'model.blend', frozen)
        assert sha(frozen) == model_hash
        notes = [
            'Texture review only; this exact geometry was already approved. Original known pixels and geometry remain preserved.',
            'The first/top-left tile is the original game orthographic camera. Actual saved materials are shown on neutral gray.',
            'Only the permitted Leicester southeast cottage and moat bank tree examples supplemented the inferred crown and bark.',
            'Brightness varies around the inferred reverse bark. Foreground foliage ownership and final scene joints remain separate unfinished work.',
        ]
        review = dest / 'receipts' / f'{asset}-review.json'
        review.write_text(json.dumps(dict(status='ready-for-user-texture-review', model_sha256=model_hash,
            root_review=receipt, self_review=json.loads((w / 'inspection/self-review.json').read_text()),
            preservation=proof, limitations=notes, texture_approved=False), indent=2) + '\n')
        validation = dest / 'receipts' / f'{asset}-validation.json'
        validation.write_text(json.dumps(dict(status='PASS', scope='guarded texture candidate and reopened review',
            model_sha256=model_hash, preservation_report_sha256=sha(b / 'reopened-preservation.json'),
            original_validation_sha256=sha(geometry / 'validation.json')), indent=2) + '\n')
        ownership = [p for p in (geometry / 'projection').glob('*/ownership.json') if p.parent.name != 'input']
        assert len(ownership) == 1
        generated = e / 'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png'
        items.append(dict(id=asset, name=name, status='ready-for-user', technical_eligible=True, model=str(frozen),
            solid=str(labeled_sheet(geometry, 'modified/solid.png')), solid_label='Already approved solid geometry',
            textured=str(labeled_sheet(w, 'inspection/actual-materials/sheet.png')),
            textured_label='Actual saved texture fill — all eight views on neutral gray',
            context=str(approved / 'modified/context.png'),
            source_comparison=str(labeled_sheet(geometry, 'modified/textured.png')),
            source_comparison_label='Approved source sheet before texture fill',
            source_comparison_secondary=str(w / 'inspection/native-source/comparison.png'),
            source_comparison_secondary_label='Original artwork, saved texture and overlay',
            source_trace=str(labeled_sheet(w, str(generated))),
            source_trace_label='Protected generated reference; actual model shown above',
            validation=str(validation), ownership=str(ownership[0]), review=str(review), notes=notes))
        archive.append(dict(asset_id=asset, model_sha256=model_hash, model=str(frozen.relative_to(dest)),
                            root_review=receipt, user_geometry_decision_sha256=sha(root / 'user-decision.json')))
    manifest = dest / 'review-candidates.json'
    manifest.write_text(json.dumps(dict(map='Crossings01 tree texture', items=items,
        status_counts={'ready for texture review': len(items), 'user texture approved': 0}), indent=2) + '\n')
    build(manifest, dest / 'gallery', pending_only=True)
    files = {str(p.relative_to(dest)): sha(p) for p in dest.rglob('*') if p.is_file()}
    (dest / 'archive.json').write_text(json.dumps(dict(status='immutable user texture review snapshot',
        assets=archive, files=files), indent=2) + '\n')
    print(dest / 'gallery/index.html')


if __name__ == '__main__':
    main()
