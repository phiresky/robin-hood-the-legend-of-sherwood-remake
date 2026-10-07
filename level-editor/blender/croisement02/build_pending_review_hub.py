"""Link immutable review batches without copying models or implying approval."""
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
from urllib.parse import quote


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('batches', nargs='+', type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    batches, sections = [], []
    for folder in args.batches:
        folder = folder.resolve()
        if (folder / 'user-approval.json').exists():
            raise ValueError(f'Already approved batch: {folder}')
        evidence = folder / 'evidence.json'
        data = json.loads(evidence.read_text())
        for relative, resource in data['resources'].items():
            assert sha(folder / relative) == resource['sha256'], relative
        index = folder / 'index.html'
        index_text = index.read_text()
        cards = []
        links = []
        for card in data['cards']:
            assert f'id="{card["card_id"]}"' in index_text, card['card_id']
            for member in card['members']:
                kind = 'artifact' if 'artifact' in member else 'model'
                assert sha(member[kind]) == member[kind + '_sha256']
                assert sha(member['source_evidence']) == member['source_evidence_sha256']
                for contract in member.get('contract_members', []):
                    assert sha(contract['contract']) == contract['contract_sha256']
            relative = os.path.relpath(index, args.output.resolve())
            link = relative + '#' + quote(card['card_id'])
            cards.append(dict(card_id=card['card_id'], scope=card['scope'],
                              link=link, members=card['members']))
            descriptions = list(dict.fromkeys(m.get('scope_description', m['scope'])
                                              for m in card['members']))
            links.append('<li><a href="' + html.escape(link, quote=True) + '">' +
                         html.escape(card['title']) + '</a> <strong>' +
                         html.escape(card['scope'].upper()) + '</strong><p>' +
                         html.escape(' '.join(descriptions)) + '</p></li>')
        batches.append(dict(evidence=str(evidence), evidence_sha256=sha(evidence),
                            index=str(index), index_sha256=sha(index), cards=cards))
        sections.append('<section><h2>' + html.escape(data['title']) + '</h2><ol>' +
                        ''.join(links) + '</ol></section>')
    count = sum(len(b['cards']) for b in batches)
    decisions = sum(len(c['members']) for b in batches for c in b['cards'])
    manifest = dict(status='Pending user review; no decisions implied',
                    card_count=count, decision_count=decisions, batches=batches,
                    resources_copied=0, models_copied=0)
    args.output.mkdir(parents=True)
    (args.output / 'evidence.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (args.output / 'index.html').write_text(
        '<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width">'
        '<title>Combined pending refinement review</title><style>'
        'body{background:#15191c;color:#eee;font:18px/1.5 system-ui;max-width:1100px;margin:40px auto;padding:20px}'
        'a{color:#abd9ff}section{border:1px solid #59616a;padding:24px;margin:30px 0}'
        'li{margin:24px 0}strong{font-size:13px;background:#3c4d65;padding:5px}p{color:#cbd0d5}'
        '</style><h1>Combined pending refinement review</h1><p>' +
        f'{count} cards, {decisions} scoped member decisions. Links open the exact frozen cards and review controls.' +
        '</p><p>Geometry, texture appearance, and source-state application are separate scopes. '
        'The source-state card binds 82 artwork contracts; it does not approve physical endpoints, motion, or gameplay. '
        'Already approved batches are excluded.</p><a href="evidence.json">Exact combined evidence manifest</a>' +
        ''.join(sections))
    print(json.dumps(dict(output=str(args.output), cards=count, decisions=decisions,
                          evidence_sha256=sha(args.output / 'evidence.json'))))


if __name__ == '__main__':
    main()
