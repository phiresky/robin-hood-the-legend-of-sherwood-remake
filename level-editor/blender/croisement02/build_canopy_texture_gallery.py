"""Build a private, hash-checked texture comparison for approved tree geometry."""
from pathlib import Path
import hashlib
import html
import json
import os

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement02-refinement'
CHOICES = {
    0: 'wood-context-preparation-v2/experiment/bake-v1',
    1: 'wood-context-preparation-v2/experiment/bake-v1',
    2: 'native-front-preparation/experiment/bake-v1',
    3: 'wood-context-preparation-v2/experiment/bake-v1',
    4: 'foliage-context-preparation-v2/experiment/bake-v1',
    5: 'native-front-preparation/experiment/bake-v1',
    6: 'native-front-preparation/experiment/bake-v1',
    8: 'native-front-preparation/experiment/bake-v1',
    10: 'native-front-preparation/experiment/bake-v1',
    11: 'wood-context-preparation-v2/experiment/retained-wood-v1',
    12: 'wood-context-preparation-v2/experiment/bake-v1',
    13: 'wood-context-preparation-v2/experiment/retained-wood-v1',
    15: 'native-front-preparation/experiment/bake-v1',
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    output = OUT / 'canopy-texture-review'
    output.mkdir(exist_ok=True)
    decisions_path = output / 'decisions.json'
    decisions = json.loads(decisions_path.read_text())['decisions'] if decisions_path.exists() else []
    rows, cards = [], []
    for number, relative in CHOICES.items():
        asset = f'croisement02-tree-{number:02d}'
        packet = OUT / 'texture-fill-round-3' / asset
        candidate = packet / relative
        review_path = candidate / 'agent-material-review.json'
        review = json.loads(review_path.read_text())
        if not review.get('ready_for_coordinator_review'):
            raise ValueError('Candidate has not passed its own material review: ' + asset)
        worker = candidate / 'worker.blend'
        actual = candidate / 'actual/textured.png'
        if sha(worker) != review['model_sha256'] or sha(actual) != review['actual_sheet_sha256']:
            raise ValueError('Reviewed candidate changed: ' + asset)
        bridge = json.loads((packet / 'approval-bridge.json').read_text())
        baseline = Path(bridge['worker']) / 'inspection/actual-materials/sheet.png'
        experiment = candidate.parent
        generation = experiment / 'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
        images = [('Approved geometry: previous material', baseline),
                  ('Texture candidate: saved model', actual),
                  ('Protected native input', experiment / 'input.png'),
                  ('Generated fill with native pixels restored', generation / 'generated-preserved.png')]
        def link(path):
            return html.escape(os.path.relpath(path, output), quote=True)
        figures = ''.join(f'<figure><figcaption>{html.escape(label)}</figcaption>'
                          f'<a href="{link(path)}"><img loading="lazy" src="{link(path)}" alt="{html.escape(label)}"></a></figure>'
                          for label, path in images)
        note = 'Original approved wood retained exactly; generated foliage only.' if number in (11, 13) else 'Geometry and native source pixels preserved; inferred texture candidate.'
        approved = any(d['asset_id'] == asset and d['decision'] == 'approved'
                       and d['model_sha256'] == sha(worker)
                       and all(Path(p).exists() and sha(Path(p)) == h for p, h in d['evidence_sha256'].items())
                       for d in decisions)
        approval_label = 'Texture approved by user.' if approved else 'Texture approval pending.'
        cards.append(f'<section id="{asset}"><h2>{asset}</h2><p>{note} {approval_label}</p>'
                     f'<div class="images">{figures}</div><p><a href="{link(review_path)}">Material review evidence</a></p></section>')
        paths = [worker, actual, baseline, review_path, experiment / 'input.png', generation / 'generated-preserved.png']
        rows.append(dict(asset_id=asset, candidate=str(candidate), model_sha256=sha(worker),
                         geometry_approval=bridge['source_decision'], texture_approval='approved' if approved else 'pending',
                         evidence={str(path): sha(path) for path in paths}))
    (output / 'evidence.json').write_text(json.dumps(dict(status='private coordinator review', items=rows), indent=2) + '\n')
    body = '<!doctype html><meta charset="utf-8"><title>Croisement02 canopy textures</title><style>body{margin:28px;background:#17191c;color:#e5e8eb;font:16px system-ui}h1,h2{font-weight:600}p{color:#bbc2cb}section{margin:36px 0;padding:20px;background:#24272c;border-radius:8px}.images{display:grid;grid-template-columns:1fr 1fr;gap:12px}figure{margin:0}figcaption{padding:8px 0}img{width:100%;background:#111}a{color:#9fcaff}@media(max-width:900px){.images{grid-template-columns:1fr}}</style><h1>Croisement02 canopy texture review</h1><p>Thirteen texture candidates on approved geometry. Each candidate has passed its own saved-model review. Exact user approval is shown per candidate; integration remains separate.</p><p>Open an image for all eight views at full size. Existing grazing-angle foliage bands belong to the approved geometry and remain visible.</p>'
    (output / 'index.html').write_text(body + '\n'.join(cards) + '\n')
    print(output / 'index.html')


if __name__ == '__main__':
    main()
