"""Freeze independently reviewed hole endpoints as one paired geometry card."""
import hashlib
import json
from pathlib import Path

OUT = Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
ROOT = OUT/'restart9-hole-endpoints'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    packet_path = ROOT/'review-packet-v1/review-packet.json'
    root_path = ROOT/'review-packet-v1/root-review.json'
    packet = json.loads(packet_path.read_text())
    root = json.loads(root_path.read_text())
    assert root['packet_sha256'] == digest(packet_path)
    assert root['status'].startswith('PASS for grouped user endpoint geometry')
    assert root['models'] == packet['models']
    evidence = dict(packet['files'])
    plan = json.loads((ROOT/'source-plan-v1/plan.json').read_text())
    for path in [packet_path, root_path, Path(packet['self_review']),
                 ROOT/'endpoint-guard-v1/aperture-disjointness.json']:
        evidence[str(path)] = digest(path)
    for row in plan['phases']:
        assert digest(Path(row['source'])) == row['sha256']
        evidence[row['source']] = row['sha256']
    for path, expected in evidence.items():
        assert digest(Path(path)) == expected, path
    items = []
    for phase, row in packet['models'].items():
        model = Path(row['path'])
        assert digest(model) == row['sha256']
        folder = model.parent
        images = [('Original artwork and modeled native view', folder/'source-comparison.png'),
                  ('Actual materials, original camera top-left', folder/'actual-sheet.png'),
                  ('Solid geometry, original camera top-left', folder/'solid-sheet.png')]
        for terrain in ['flat', 'bank']:
            contact = ROOT/f'contact-v1/{terrain}-{phase}'
            images += [(f'{terrain.title()} terrain contact, original camera top-left', contact/'contact8.png'),
                       (f'{terrain.title()} source comparison; surrounding foliage omitted', contact/'source-comparison.png')]
        displayed = [dict(label=label,file=str(path),sha256=digest(path)) for label,path in images]
        assert all(image['file'] in evidence for image in displayed)
        notes = list(packet['notes'])
        notes.append('Geometry review only. Runtime aperture integration and future unknown-surface appearance are separate; no gameplay masks, timing, or native state transitions are changed.')
        asset = 'croisement02-hole-'+phase
        revision = hashlib.sha256(json.dumps(dict(asset_id=asset,model_sha256=row['sha256'],
                    evidence=evidence,displayed_images=displayed,notes=notes),sort_keys=True,separators=(',',':')).encode()).hexdigest()
        items.append(dict(asset_id=asset,name='Leaf-covered trap — '+('initial cover' if phase=='initial' else 'applied recess'),
                    model_sha256=row['sha256'],review_revision=revision,decision='pending',
                    scope='geometry',worker=str(folder),evidence=evidence,displayed_images=displayed,notes=notes,
                    canonical_profile=dict(profile='Croisement01 - hole',phase=phase,
                                           mission_instances=30,unique_display_positions=17)))
    dest = ROOT/'paired-geometry-ready-v1'
    dest.mkdir(exist_ok=False)
    result = dict(status='Root-reviewed endpoint geometry; user decisions pending',items=items,
                  cards=[dict(card_id='croisement02-hole-endpoint-pair',title='Leaf-covered trap — cover and opened recess',
                              asset_ids=[row['asset_id'] for row in items])],canonical_mutation=False)
    path = dest/'bound-members.json'
    path.write_text(json.dumps(result,indent=2)+'\n')
    print(path, digest(path))


if __name__ == '__main__':
    main()
