"""Archive the explicit four-card texture approval against frozen evidence."""
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from texture_decisions import evidence, fields, sha


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def main():
    root = ROOT / 'level-editor/work/croisement03-refinement/restart2'
    reviewed = root / 'texture-review-round1'
    frozen = json.loads((reviewed / 'freeze.json').read_text())
    assert sha(reviewed / 'texture-candidates.json') == frozen['manifest_sha256']
    for relative, digest in frozen['gallery_files'].items():
        assert sha(reviewed / relative) == digest, relative
    displayed = json.loads((reviewed / 'gallery/evidence.json').read_text())['items']
    assert len(displayed) == 4
    archive = root / 'texture-approval-round1'
    archive.mkdir(exist_ok=False)
    shutil.copytree(reviewed / 'gallery', archive / 'gallery')
    for filename in ['freeze.json', 'texture-candidates.json']:
        shutil.copyfile(reviewed / filename, archive / filename)
    batch = dict(scope='texture', decision='approved', approved_by='user',
                 exact_user_text='All four textures approved',
                 authorization='Explicit user approval for the frozen four-card texture-review-round1, relayed by root.',
                 freeze_sha256=sha(archive / 'freeze.json'),
                 limitation='Isolated appearances approved; surrounding scene and final map integration remain unfinished.')
    write(archive / 'decision.json', batch)
    decisions = []
    for item in displayed:
        paths, hashes = evidence(item)
        images, reports = fields(item)
        for key in (*images, *reports):
            entry = item['images' if key in images else 'reports'][key]
            assert hashes[key] == entry['sha256']
            assert sha(reviewed / 'gallery' / entry['file']) == entry['sha256']
        destination = archive / 'assets' / item['id'] / item['review_revision']
        destination.mkdir(parents=True)
        for key, source in paths.items():
            shutil.copy2(source, destination / (key + source.suffix))
        decision = dict(asset_id=item['id'], scope='texture', decision='approved',
                        review_revision=item['review_revision'],
                        exact_user_text=batch['exact_user_text'],
                        batch_decision=str(archive / 'decision.json'),
                        evidence_sha256=hashes,
                        evidence_paths={key: str(path) for key, path in paths.items()},
                        archive=str(destination))
        write(destination / 'decision.json', decision)
        decisions.append(decision)
    write(reviewed / 'decisions.json', dict(version=1, decisions=decisions))
    write(archive / 'decisions.json', dict(version=1, decisions=decisions))
    print(json.dumps(dict(approved=len(decisions), archive=str(archive))))


if __name__ == '__main__':
    main()
