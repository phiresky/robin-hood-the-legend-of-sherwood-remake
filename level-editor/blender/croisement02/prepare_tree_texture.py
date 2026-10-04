"""Adapt archived Croisement02 approvals to immutable shared texture packets.

This is a schema bridge, not an approval or a rendering step. Every reviewed
pixel and camera is copied unchanged; technical eligibility is rechecked from
saved audits and the manual review that preceded the actual user decision.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / 'refinement'))
from catalog import OUT, tree_workspace, scenery_workspace
from prepare_texture_packet import prepare
from review_evidence import sha

PERMITTED = ('leicester-southeast-cottage-tree', 'leicester-moat-bank-tree')


def read(path):
    return json.loads(Path(path).read_text())


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def checked(path, digest):
    path = Path(path).resolve(strict=True)
    require(sha(path) == digest, 'Changed evidence: ' + str(path))
    return path


def prepare_tree(number, destination):
    asset = f'croisement02-tree-{number:02d}'
    return prepare_asset(asset, tree_workspace(number), destination, tree=True)


def prepare_asset(asset, worker, destination, *, tree=False):
    records = [r for r in read(OUT / 'user-feedback.json')['records'] if r['asset_id'] == asset]
    require(bool(records), 'No explicit user decision')
    decision = records[-1]
    require(decision['decision'] == 'approved' and decision['scope'] == 'geometry', 'Geometry is not approved')
    archive = Path(decision['archive'])
    require(read(archive / 'decision.json') == decision, 'Archived decision differs')
    archived = read(archive / 'gallery-item.json')
    require(archived['status'] == 'ready-for-user' and archived['technical_eligible'], 'Archived review was incomplete')
    model_hash = decision['model_sha256']
    checked(worker / 'model.blend', model_hash)
    checked(archive / 'model.blend', model_hash)
    binding = {'model': model_hash}
    for category in ('images', 'reports'):
        binding[category] = {key: entry['sha256'] for key, entry in archived[category].items()}
        for entry in archived[category].values():
            checked(archive / entry['file'], entry['sha256'])
    digest = hashlib.sha256(json.dumps(binding, sort_keys=True).encode()).hexdigest()
    require(digest == decision['review_revision'] == archived['review_revision'], 'Archived revision is not the approved revision')
    for name in ('solid', 'textured'):
        checked(archive / f'modified/{name}.png', decision[name + '_sha256'])
    for path in (archive / 'modified').rglob('*'):
        if path.is_file():
            checked(worker / path.relative_to(archive), sha(path))
    validation = read(worker / 'validation.json')
    audit = read(worker / 'inspection/saved-model-audit.json')
    visual = read(worker / 'inspection/visual-review.json')
    actual = read(worker / 'inspection/actual-materials/evidence.json')
    require(validation['status'] == audit['status'] == 'PASS', 'Failed saved-model validation')
    require(all(record['model_sha256'] == model_hash for record in (audit, visual, actual)), 'Stale technical or visual evidence')
    require(visual['ready_for_geometry_review'], 'Manual visual review incomplete')
    if tree:
        coverage = read(worker / 'inspection/source-coverage/report.json')
        bounds = read(worker / 'inspection/actual-materials/opacity-bounds.json')
        require(coverage['model_sha256'] == model_hash, 'Stale source coverage')
        require(coverage['intersection_over_union'] >= .95, 'Source silhouette coverage failed')
        require(min(c['depth_width_ratio'] for c in bounds['crowns']) >= 1, 'Crown depth failed')
    checked(worker / audit['inspection_recipe']['recipe'], audit['inspection_recipe']['recipe_sha256'])
    checked(worker / 'inspection/actual-materials/sheet.png', actual['sheet_sha256'])
    visual_sheet = visual.get('sheet_sha256')
    if visual_sheet is None:
        visual_sheet = visual['reviewed_images'][f'assets/{asset}/inspection/actual-materials/sheet.png']
    checked(worker / 'inspection/actual-materials/sheet.png', visual_sheet)
    if visual.get('self_review_packet'):
        checked(visual['self_review_packet'], visual['self_review_packet_sha256'])
    if visual.get('full_crown_evidence_sha256'):
        checked(worker / 'inspection/full-crown/evidence.json', visual['full_crown_evidence_sha256'])
    if visual.get('preservation_evidence'):
        checked(visual['preservation_evidence'], visual['preservation_evidence_sha256'])
    # The archived gallery already bound the actual material image and audit.
    checked(worker / 'inspection/saved-model-audit.json', archived['reports']['stored_material_audit']['sha256'])
    checked(worker / 'inspection/actual-materials/sheet.png', archived['images']['stored_material_textured']['sha256'])
    frames = read(archive / 'modified/views.json')
    for path, digest in frames['source_mask_evidence'].items():
        checked(path, digest)
    require(not destination.exists(), 'Output already exists; use a new experiment directory')
    destination.mkdir(parents=True)
    evidence = {}

    def bind(key, path):
        path = Path(path).resolve(strict=True)
        evidence[key] = {'path': str(path), 'sha256': sha(path)}

    # These files are in the immutable user archive, including genuine per-view
    # Source-ownership buffers. Bind every file rather than just the display PNGs.
    for path in sorted(archive.rglob('*')):
        if path.is_file():
            bind('approved/' + str(path.relative_to(archive)), path)
    proof = destination / 'technical-evidence'
    proof.mkdir()
    for relative in ('validation.json', 'inspection/saved-model-audit.json',
                     'inspection/visual-review.json', 'inspection/actual-materials/evidence.json',
                     'inspection/actual-materials/opacity-bounds.json', 'inspection/source-coverage/report.json'):
        if not tree and relative in ('inspection/actual-materials/opacity-bounds.json', 'inspection/source-coverage/report.json'):
            continue
        target = proof / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(worker / relative, target)
        bind('technical/' + relative, target)
    for index, (path, digest) in enumerate(sorted(frames['source_mask_evidence'].items())):
        bind('native-mask-' + str(index), checked(path, digest))
    bridge = dict(version=1, kind='schema-translation-of-existing-user-decision',
                  source_decision=decision, source_archive=str(archive), worker=str(worker),
                  original_gallery_revision=decision['review_revision'],
                  unchanged_model=True, unchanged_review_pixels_and_cameras=True,
                  material_validation_basis='Saved-model structural PASS and current manually reviewed actual-material evidence; no texture approval implied.')
    write(destination / 'approval-bridge.json', bridge)
    bind('approval-bridge', destination / 'approval-bridge.json')
    transport = {}
    if frames['tile_size'] == [256, 256]:
        transport = {'transport_padding': {'version': 1, 'kind': 'bottom-padding',
            'width': 1024, 'height': 640,
            'content_box': {'left': 0, 'top': 0, 'width': 1024, 'height': 512}}}
        selection = destination / 'transport-selection.json'
        write(selection, transport)
        bind('transport-selection', selection)
        transport['preparation_selection'] = str(selection)
        # Transport padding does not change the frozen content, camera, or mask.
        # It is a preparation choice, not additional user-reviewed geometry.
    identity = dict(asset_id=asset, model_sha256=model_hash,
                    evidence={key: entry['sha256'] for key, entry in evidence.items()})
    revision = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    item = dict(id=asset, workspace=str(archive), status='ready-for-user',
                stored_material_validation='PASS', solid=str(archive / 'modified/solid.png'),
                textured=str(archive / 'modified/textured.png'),
                revision=dict(sha256=revision, model_sha256=model_hash, evidence=evidence),
                approval_provenance=bridge, **transport)
    translated = dict(asset_id=asset, scope='geometry', decision='approved',
                      exact_user_text=decision['exact_user_text'], revision_sha256=revision,
                      original_gallery_decision=decision,
                      translation='Same approved model, source images, cameras, and ownership buffers; schema adaptation only.')
    manifest = destination / 'review-manifest.json'
    decisions = destination / 'decisions.json'
    write(manifest, dict(version=1, items=[item]))
    write(decisions, dict(version=1, decisions=[translated]))
    result = prepare(manifest, asset, destination / 'experiment', decisions)
    experiment = Path(result['output'])
    if tree:
        attach_references(experiment)
    return result


def attach_references(experiment):
    """Bind the two user-designated examples, without extending old approvals."""
    references = []
    reference_root = OUT.parent / 'leicester-refinement/round-1/texture-review/approved-evidence'
    (experiment / 'material-references').mkdir(exist_ok=True)
    for reference in PERMITTED:
        choices = list((reference_root / reference).glob('*/decision.json'))
        require(len(choices) == 1, 'Expected one explicit approved reference revision')
        source_decision = read(choices[0])
        require(source_decision['decision'] == 'approved' and source_decision['scope'] == 'texture', 'Reference texture is not approved')
        source = (choices[0].parent / 'textured.png').resolve(strict=True)
        historic_hash = source_decision['evidence_sha256']['textured']
        copied = experiment / 'material-references' / (reference + '.png')
        shutil.copyfile(source, copied)
        shutil.copyfile(choices[0], copied.with_suffix('.decision.json'))
        references.append(dict(source='material', file=str(copied), sha256=sha(copied), asset_id=reference,
                               role='Supplementary bark and leaf texture/style only. Ignore gray unknown patches; preserve target geometry, cameras, lighting, alpha, and known source pixels.',
                               historic_approval_decision=str(copied.with_suffix('.decision.json')),
                               historic_approval_decision_sha256=sha(copied.with_suffix('.decision.json')),
                               historic_approved_image_sha256=historic_hash,
                               matches_historic_approved_bytes=sha(copied) == historic_hash,
                               authorization='User explicitly designated this named tree as a supplementary example; historic texture approval is not extended to changed bytes.'))
    write(experiment / 'auxiliary-references.json', dict(version=1, input_sha256=sha(experiment / 'input.png'),
          lighting_sha256=sha(experiment / 'solid.png'), references=references))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trees', nargs='*', type=int)
    parser.add_argument('--scenery', nargs='*', default=[], help='Explicit scenery asset IDs; no tree material references added')
    parser.add_argument('--output-root', type=Path, default=OUT / 'texture-fill-round-1')
    args = parser.parse_args()
    for number in args.trees:
        print(json.dumps(prepare_tree(number, args.output_root / f'croisement02-tree-{number:02d}')), flush=True)
    for asset in args.scenery:
        print(json.dumps(prepare_asset(asset, scenery_workspace(asset), args.output_root / asset)), flush=True)
