"""Freeze reviewed canopy texture cards without changing historical galleries."""
import argparse
import hashlib
import html
import json
from pathlib import Path
import shutil


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(input_root, trees, output):
    if output.exists():
        raise FileExistsError(output)
    prepared = []
    for number in trees:
        asset = f'croisement02-tree-{number:02d}'
        packet = input_root / asset
        candidate = packet / 'native-front-preparation/experiment/retained-wood-v1'
        experiment = candidate.parent
        bridge = json.loads((packet / 'approval-bridge.json').read_text())
        proof = json.loads((candidate / 'preservation.json').read_text())
        root = json.loads((candidate / 'root-review.json').read_text())
        worker, actual = candidate / 'worker.blend', candidate / 'actual/textured.png'
        if (root['status'] != 'PASS' or not root['all_eight_saved_model_views_inspected']
                or root['model_sha256'] != sha(worker) or root['actual_sheet_sha256'] != sha(actual)
                or root['preservation_report_sha256'] != sha(candidate / 'preservation.json')
                or proof['original_model_sha256'] != bridge['source_decision']['model_sha256']
                or proof['candidate_model_sha256'] != sha(worker)):
            raise ValueError('Root review or approved geometry binding changed: ' + asset)
        baseline = Path(bridge['worker']) / 'inspection/actual-materials/sheet.png'
        generated = experiment / 'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png'
        images = [('Approved geometry: previous material', baseline),
                  ('New texture on saved model: all eight views', actual),
                  ('Protected native input', experiment / 'input.png'),
                  ('Generated fill with native pixels restored', generated)]
        paths = [worker, *[p for _, p in images], candidate / 'agent-material-review.json',
                 candidate / 'root-review.json', candidate / 'preservation.json',
                 experiment / 'bake-v1/reopened-preservation.json', packet / 'approval-bridge.json']
        hashes = {str(p): sha(p) for p in paths}
        revision = hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest()
        row = dict(asset_id=asset, candidate=str(candidate), model_sha256=sha(worker),
                   geometry_approval=bridge['source_decision'], texture_approval='pending',
                   review_revision=revision, evidence=hashes)
        prepared.append((number, row, images))
    output.mkdir(parents=True)
    (output / 'images').mkdir()
    cards, rows = [], []
    for number, row, images in prepared:
        figures, frozen = [], []
        for index, (label, source) in enumerate(images):
            name = f'images/{row["asset_id"]}-{index}-{sha(source)[:16]}.png'
            shutil.copyfile(source, output / name)
            if sha(output / name) != row['evidence'][str(source)]:
                raise ValueError('Displayed evidence changed while freezing')
            frozen.append(dict(label=label, file=name, source=str(source), sha256=sha(source)))
            figures.append(f'<figure><figcaption>{html.escape(label)}</figcaption>'
                           f'<a href="{name}"><img loading="lazy" src="{name}"></a></figure>')
        note = 'Approved wood and native leaf pixels remain exact. Existing grazing leaf-card edges remain.'
        if number in (14, 17):
            note += ' The earlier ringlike bark is retained unchanged.'
        if number == 40:
            note += ' The coarse sparse native front versus dense inferred rear transition is particularly visible.'
        cards.append(f'<section><h2>{row["asset_id"]}</h2><p>Texture approval pending. {note}</p>'
                     '<div class="images">' + ''.join(figures) + '</div></section>')
        rows.append(dict(row, displayed_images=frozen, notes=note))
    document = dict(status='frozen pending user texture review', items=rows,
                    scope='Only these exact new textures; historical approved canopy gallery excluded')
    (output / 'evidence.json').write_text(json.dumps(document, indent=2) + '\n')
    body = ('<!doctype html><meta charset="utf-8"><title>Croisement02 new canopy textures</title>'
            '<style>body{margin:28px;background:#17191c;color:#eee;font:16px system-ui}'
            'section{margin:30px 0;padding:18px;background:#24272c}.images{display:grid;grid-template-columns:1fr 1fr;gap:12px}'
            'figure{margin:0}img{width:100%}figcaption{padding:8px 0}@media(max-width:900px){.images{grid-template-columns:1fr}}</style>'
            f'<h1>Croisement02: {len(rows)} new canopy textures</h1><p>Geometry was approved. These exact texture candidates passed independent saved-model review and await your texture approval. Open any image for all eight views.</p>')
    (output / 'index.html').write_text(body + ''.join(cards))
    print(output / 'index.html')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input-root', type=Path, required=True)
    parser.add_argument('--trees', type=int, nargs='+', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    build(args.input_root.resolve(), args.trees, args.output.resolve())
