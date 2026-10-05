"""Strict selection of separately reviewed additive prop completions."""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json
from catalog_schema import source_for_part

ASSETS = {'croisement02-woodcutters-shed', 'croisement02-logging-clearing-log'}


def validate(worker, group):
    model = sha(worker / 'model.blend')
    audit = json.loads((worker / 'inspection/saved-model-audit.json').read_text())
    visual = json.loads((worker / 'inspection/visual-review.json').read_text())
    root = json.loads((worker / 'inspection/root-completion-review.json').read_text())
    joint = json.loads((worker / 'inspection/joint-neighbourhood.json').read_text())
    if any(r['model_sha256'] != model for r in (audit, visual, root, joint)):
        raise ValueError('Prop completion model binding changed')
    if audit['status'] != 'PASS' or json.loads((worker / 'validation.json').read_text())['status'] != 'PASS':
        raise ValueError('Prop completion validation failed')
    if not visual['ready_for_geometry_review'] or root['status'] != 'PASS':
        raise ValueError('Prop completion reviews incomplete')
    expected = {source_for_part(p) for p in group['parts']}
    if {r['source_node'] for r in audit['objects']} != expected:
        raise ValueError('Prop completion source identities changed')
    for key in ('sheet', 'evidence'):
        if sha(Path(joint[key])) != joint[key + '_sha256']:
            raise ValueError('Prop completion joint evidence changed')
    if sha(worker / 'inspection/actual-materials/sheet.png') != visual['actual_materials_sha256']:
        raise ValueError('Prop completion actual views changed')
    return model


def selected_workspace(out, asset, catalog):
    if asset not in ASSETS:
        return None
    path = out / 'restart2-vegetation/prop-selections' / (asset + '.json')
    if not path.exists():
        return None
    receipt = json.loads(path.read_text())
    group = next(g for g in json.loads(catalog.read_text())['groups'] if g['id'] == asset)
    if receipt['group'] != group or receipt['asset_id'] != asset:
        raise ValueError('Prop completion catalog binding changed')
    for name, digest in receipt['files'].items():
        if sha(Path(name)) != digest:
            raise ValueError('Prop completion evidence changed: ' + name)
    worker = Path(receipt['worker'])
    if validate(worker, group) != receipt['model_sha256']:
        raise ValueError('Prop completion selected model changed')
    return worker


def expose(worker):
    from catalog import OUT, reviewed_catalog
    if worker.name not in ASSETS:
        raise ValueError('Unsupported prop completion')
    group = next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id'] == worker.name)
    model = validate(worker, group)
    target = OUT / 'restart2-vegetation/prop-selections' / (worker.name + '.json')
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        raise FileExistsError('Archive the previous selection before replacing it')
    paths = [worker / n for n in ('model.blend', 'workspace.json', 'source-masks.json', 'validation.json')]
    for folder in ('inspection', 'input', 'modified', 'reference'):
        paths.extend(p for p in (worker / folder).rglob('*') if p.is_file())
    joint = json.loads((worker / 'inspection/joint-neighbourhood.json').read_text())
    paths.extend(Path(joint[k]) for k in ('sheet', 'evidence'))
    write_json(target, dict(asset_id=worker.name, worker=str(worker), group=group,
                           model_sha256=model, approval='pending new geometry decision',
                           files={str(p): sha(p) for p in paths}))
