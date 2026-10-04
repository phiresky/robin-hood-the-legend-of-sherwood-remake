"""Bind the reviewed east-rail source-appearance correction to approved geometry.

This narrow bridge retains the original decision and independently verified
receiver geometry. Corrected source pixels are preparation evidence, not a new
user texture approval. Other assets still use the strict unchanged-model path.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from prepare_tree_texture import OUT, checked, prepare, read, require, sha, write
from catalog import scenery_workspace


def main(proof_path, review_path, output):
    asset = 'croisement02-east-rail-fence'
    worker = scenery_workspace(asset)
    decision = [r for r in read(OUT / 'user-feedback.json')['records'] if r['asset_id'] == asset][-1]
    require(decision['decision'] == 'approved' and decision['scope'] == 'geometry', 'Geometry decision required')
    archive = Path(decision['archive'])
    require(read(archive / 'decision.json') == decision, 'Archived decision changed')
    archived = read(archive / 'gallery-item.json')
    binding = {'model': decision['model_sha256']}
    for category in ('images', 'reports'):
        binding[category] = {k: v['sha256'] for k, v in archived[category].items()}
        for entry in archived[category].values():
            checked(archive / entry['file'], entry['sha256'])
    require(hashlib.sha256(json.dumps(binding, sort_keys=True).encode()).hexdigest() == decision['review_revision'], 'Original review revision changed')
    receipt_path = worker / 'inspection/feedback-revision-1.json'
    receipt, proof, review = read(receipt_path), read(proof_path), read(review_path)
    checked(archive / 'model.blend', decision['model_sha256'])
    checked(worker / 'model.blend', receipt['model_sha256'])
    require(receipt['before_model_sha256'] == decision['model_sha256'], 'Correction has a different approval parent')
    require(receipt['geometry_changed'] is False and receipt['before_geometry_sha256'] == receipt['geometry_sha256'], 'Correction changed geometry')
    require(proof['geometry_identical'] is True, 'Independent geometry comparison failed')
    for label, model_hash in [('approved', decision['model_sha256']), ('current', receipt['model_sha256'])]:
        require(proof[label]['model_sha256'] == model_hash and proof[label]['geometry_sha256'] == receipt['geometry_sha256'] and proof[label]['mesh_count'] == 2, 'Independent proof does not bind this correction')
    require(review['asset_id'] == asset and review['ready_for_texture_preparation'] is True and bool(review['reviewer']), 'Corrected source review required')
    checked(receipt_path, review['correction_sha256'])
    validation = read(worker / 'validation.json')
    audit = read(worker / 'inspection/saved-model-audit.json')
    visual = read(worker / 'inspection/visual-review.json')
    actual = read(worker / 'inspection/actual-materials/evidence.json')
    require(validation['status'] == audit['status'] == 'PASS', 'Saved model validation failed')
    require(all(r['model_sha256'] == receipt['model_sha256'] for r in (audit, visual, actual)), 'Stale corrected model evidence')
    require(visual['ready_for_geometry_review'], 'Corrected actual materials unreviewed')
    checked(worker / 'inspection/actual-materials/sheet.png', visual['sheet_sha256'])
    checked(worker / 'inspection/actual-materials/sheet.png', actual['sheet_sha256'])
    checked(worker / audit['inspection_recipe']['recipe'], audit['inspection_recipe']['recipe_sha256'])
    frames = read(worker / 'modified/views.json')
    for path, digest in frames['source_mask_evidence'].items():
        checked(path, digest)
    require(not output.exists(), 'Use a new immutable output directory')
    output.mkdir(parents=True)
    snapshot = output / 'corrected-source'
    snapshot.mkdir()
    shutil.copyfile(worker / 'model.blend', snapshot / 'model.blend')
    for directory in ('modified', 'inspection'):
        shutil.copytree(worker / directory, snapshot / directory)
    shutil.copyfile(worker / 'validation.json', snapshot / 'validation.json')
    shutil.copyfile(proof_path, snapshot / 'geometry-comparison.json')
    shutil.copyfile(review_path, snapshot / 'input-review.json')
    provenance = dict(kind='appearance-only-source-correction', original_geometry_decision=decision,
        correction=receipt, independent_geometry_comparison=proof, input_review=review,
        scope='Same independently verified approved receiver geometry; corrected source pixels and model materials are reviewed preparation derivatives, not newly user-approved texture pixels.')
    write(output / 'approval-bridge.json', provenance)
    evidence = {}
    for prefix, root in [('approved', archive), ('corrected', snapshot)]:
        for path in sorted(root.rglob('*')):
            if path.is_file():
                evidence[prefix + '/' + str(path.relative_to(root))] = dict(path=str(path.resolve()), sha256=sha(path))
    evidence['bridge'] = dict(path=str(output / 'approval-bridge.json'), sha256=sha(output / 'approval-bridge.json'))
    for i, (path, digest) in enumerate(sorted(frames['source_mask_evidence'].items())):
        evidence['source-mask-' + str(i)] = dict(path=str(Path(path).resolve()), sha256=digest)
    padding = dict(version=1, kind='bottom-padding', width=1024, height=640,
        content_box=dict(left=0, top=0, width=1024, height=512))
    selection = output / 'transport-selection.json'
    write(selection, dict(transport_padding=padding))
    evidence['transport-selection'] = dict(path=str(selection), sha256=sha(selection))
    identity = dict(asset_id=asset, model_sha256=receipt['model_sha256'], evidence={k: v['sha256'] for k, v in evidence.items()})
    revision = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    require(frames['tile_size'] == [256, 256], 'Unexpected transport canvas')
    item = dict(id=asset, workspace=str(snapshot), status='ready-for-user', stored_material_validation='PASS',
        solid=str(snapshot / 'modified/solid.png'), textured=str(snapshot / 'modified/textured.png'),
        revision=dict(sha256=revision, model_sha256=receipt['model_sha256'], evidence=evidence),
        approval_provenance=provenance,
        transport_padding=padding, preparation_selection=str(selection))
    translated = dict(asset_id=asset, scope='geometry', decision='approved', exact_user_text=decision['exact_user_text'],
        revision_sha256=revision, original_gallery_decision=decision,
        translation='Original approved geometry independently verified unchanged; source-appearance correction reviewed separately for preparation.')
    write(output / 'review-manifest.json', dict(version=1, items=[item]))
    write(output / 'decisions.json', dict(version=1, decisions=[translated]))
    print(json.dumps(prepare(output / 'review-manifest.json', asset, output / 'experiment', output / 'decisions.json')), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('geometry_comparison', type=Path)
    parser.add_argument('input_review', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    main(args.geometry_comparison.resolve(), args.input_review.resolve(), args.output.resolve())
