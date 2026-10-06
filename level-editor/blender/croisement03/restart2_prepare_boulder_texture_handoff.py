"""Bind the consolidated boulder texture approval to identical shared staging evidence."""
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from texture_decisions import evidence, fields
from texture_staging import validate_texture_handoff

R = ROOT / 'level-editor/work/croisement03-refinement/restart2'
ASSET = 'croisement03-central-shrub-boulder'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    batch = ROOT / 'level-editor/work/croisement02-refinement/restart3-review-batches/batch-v8'
    receipt = batch / 'user-approval.json'
    assert sha(receipt) == 'd68ce43c77a2a6325c642e452eb135389c253a59e83de3e937928d0c12c7c79e'
    approval = json.loads(receipt.read_text())
    assert sha(batch / 'evidence.json') == approval['evidence_sha256']
    displayed = json.loads((batch / 'evidence.json').read_text())
    card = next(card for card in displayed['cards'] if card['card_id'] == 'texture-' + ASSET + '-texture')
    member = card['members'][0]
    decision = next(row for row in approval['decisions'] if row['card_id'] == card['card_id'])
    assert member['model_sha256'] == decision['members'][0]['model_sha256']
    assert member['review_revision'] == decision['members'][0]['review_revision']
    assert sha(member['model']) == member['model_sha256']
    assert sha(member['source_evidence']) == member['source_evidence_sha256']
    bound = {}
    for record in member['images'] + member['reports']:
        assert sha(batch / record['file']) == record['sha256']
        assert sha(record['source']) == record['sha256']
        bound[str(Path(record['source']).resolve())] = record['sha256']
    item = json.loads((R / 'texture-review-boulder-v2/texture-candidates.json').read_text())['items'][0]
    paths, hashes = evidence(item)
    images, reports = fields(item)
    for field in (*images, *reports):
        assert bound[str(paths[field].resolve())] == hashes[field], field
    assert hashes['model'] == member['model_sha256']
    binding = {'images': {key: hashes[key] for key in images},
               'reports': {key: hashes[key] for key in reports}}
    revision = hashlib.sha256(json.dumps(binding, sort_keys=True).encode()).hexdigest()
    output = R / 'approved-stone-integration-preflight/boulder-texture-handoff'
    output.mkdir(parents=True,exist_ok=False)
    record = dict(asset_id=ASSET, scope='texture', decision='approved',
                  exact_user_text=approval['answer'], review_revision=revision,
                  evidence_sha256=hashes, evidence_paths={key: str(path) for key, path in paths.items()},
                  original_batch_revision=member['review_revision'], batch_receipt=str(receipt),
                  batch_receipt_sha256=sha(receipt),
                  translation='Identical model and all displayed shared texture fields hash-verified against consolidated resources. Different wrapper identity only; stone appearance scope unchanged.')
    write(output / 'texture-decisions.json', {'version': 1, 'decisions': [record]})
    source = R / 'central-boulder-v2/assets' / ASSET / 'workspace.json'
    destination = R / 'geometry-approval-v7-boulder/asset/workspace.json'
    if destination.exists():
        assert sha(source) == sha(destination)
    else:
        shutil.copyfile(source, destination)
    write(output / 'workspace-metadata.json', dict(source=str(source), destination=str(destination),
          sha256=sha(source), scope='Exact original worker configuration; approved model/cameras unchanged.'))
    packet = R / 'texture-batch-v7' / ASSET
    handoff = validate_texture_handoff(packet / 'review-manifest.json', ASSET,
                                      output / 'texture-decisions.json', packet / 'decisions.json')
    write(output / 'handoff.json', handoff)
    write(output / 'receipt.json', dict(status='PASS private approval handoff; Blender geometry and export checks still required',
          batch_receipt_sha256=sha(receipt), handoff_sha256=sha(output / 'handoff.json'),
          model_sha256=member['model_sha256'], live_library_changed=False))
    print(output / 'handoff.json')


if __name__ == '__main__':
    main()
