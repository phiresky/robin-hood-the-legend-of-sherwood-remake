"""Adapt reviewed archer pairs and York jamb into the shared grouped-card format."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work'
OUT = WORK / 'croisement02-refinement/restart3-review-batches/next-pool-v19'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    if OUT.exists():
        raise ValueError('Keep existing frozen adapters unchanged')
    base = WORK / 'croisement02-refinement/restart8-hidden-archer-leaf-trial-v3'
    author_path = base / 'author-review-v1.json'
    review_path = base / 'root-review-v1.json'
    author = json.loads(author_path.read_text())
    review = json.loads(review_path.read_text())
    if not review['status'].startswith('PASS') or review['author_review_sha256'] != sha(author_path):
        raise ValueError('Exact independent archer review required')
    for filename, expected in author['files'].items():
        if sha(filename) != expected:
            raise ValueError('Archer evidence changed: ' + filename)
    items, cards = [], []
    for row in author['records']:
        model = Path(row['model'])
        if sha(model) != row['model_sha256']:
            raise ValueError('Archer model changed')
        asset = f"croisement02-hidden-archer-{row['profile']:02d}-{row['state']}"
        # Keep each model binding unique; the author receipt binds the other
        # pair members separately, while all diagnostic evidence stays linked.
        evidence = {str(model): row['model_sha256'], **author['files'],
                    str(author_path): sha(author_path), str(review_path): sha(review_path)}
        revision = hashlib.sha256(json.dumps(evidence, sort_keys=True).encode()).hexdigest()
        labels = ['Actual eight views; native first', 'Solid eight views; native first',
                  'Native artwork and current context', 'Full neighbor context, eight views',
                  'Contact diagnostic with crowns hidden, eight views']
        items.append(dict(asset_id=asset, name=f"Hidden archer {row['profile']:02d}: {row['state']}",
                          model_sha256=row['model_sha256'], review_revision=revision,
                          decision='pending', state=row['state'],
                          scope='Endpoint foliage geometry only. Appearance, intermediate motion, runtime and context repairs remain separate.',
                          evidence=evidence, notes=author['limits'],
                          displayed_images=[dict(label=label, file=name, sha256=author['files'][name])
                                            for label, name in zip(labels, row['images'], strict=True)]))
    for profile in (3, 4):
        cards.append(dict(card_id=f'hidden-archer-{profile:02d}-paired-geometry',
                          title=f'Crossings02 hidden archer {profile:02d}: both endpoint shapes',
                          asset_ids=[f'croisement02-hidden-archer-{profile:02d}-{state}'
                                     for state in ('initial', 'applied')]))

    source = WORK / 'york-refinement/restart2/jamb-textures-v1/gallery-freeze.json'
    jamb = json.loads(source.read_text())
    for item in jamb['items']:
        for filename, expected in item['evidence'].items():
            if sha(filename) != expected:
                raise ValueError('Jamb evidence changed: ' + filename)
        item['scope'] = item['scope_description']
        item['name'] = 'York gate recessed stone jamb texture'
        item['evidence'][str(source)] = sha(source)
        item['review_revision'] = hashlib.sha256(json.dumps(item['evidence'], sort_keys=True).encode()).hexdigest()
    jamb['cards'] = [dict(card_id=card['asset_id'], title=card['title'], asset_ids=card['members'])
                     for card in jamb['cards']]
    OUT.mkdir()
    write(OUT / 'archer-bound-members.json', dict(items=items, cards=cards))
    write(OUT / 'jamb-bound-members.json', jamb)
    write(OUT / 'status.json', dict(status='Frozen adapters; collecting next grouped review',
                                  archer_cards=2, archer_decisions=4, jamb_cards=1,
                                  user_approved=False, canonical_changes=False))
    print(OUT)


if __name__ == '__main__':
    main()
