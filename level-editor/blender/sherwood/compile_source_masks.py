"""Compile inspected Sherwood ownership rules with explicit unknown coverage.

An unresolved receiver gets a black bitmap. The shared source-mask API otherwise
leaves missing targets unconstrained, which is never acceptable for this scene.
This can produce an audit preview before all assignments are resolved; the report
explicitly prevents that preview from being used as a completed synthesis source.
"""
import argparse
import hashlib
import json
import shutil
from pathlib import Path
import sys

import numpy as np
from PIL import Image

EDITOR = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(EDITOR/'refinement/blender'))
from occlusion_constraints import SourceMaskConstraints, evidence_record

DEFAULT_RECIPE = Path(__file__).with_name('source-mask-candidates.json')
DEFAULT_GROUPING = EDITOR/'work/needs-review/sherwood-refinement/grouping-review'
NATIVE = EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.d/masks/manifest.json'
SOURCE = EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Day/sherwood.map.png'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2)+'\n')


def load_image(path):
    return np.asarray(Image.open(path).convert('L')) > 0


def effective_rule(candidate, reason=None):
    node = candidate['source_node']
    if candidate.get('reviewed') is not True or reason:
        return dict(source_node=node, reviewed=True, mask_indices=[10000],
                    ownership_status='unresolved', reason=reason or 'Assignment not inspected')
    if not isinstance(candidate.get('inspection'), dict):
        raise ValueError('Missing assignment inspection evidence: '+node)
    if candidate['inspection'].get('source_sha256') != sha(SOURCE):
        raise ValueError('Inspected artwork changed: '+node)
    if candidate['inspection'].get('inventory_sha256') != sha(NATIVE):
        raise ValueError('Inspected native inventory changed: '+node)
    inspected = candidate['inspection'].get('mask_sha256', {})
    card = candidate['inspection'].get('card')
    if not card or sha(EDITOR/card) != candidate['inspection'].get('card_sha256'):
        raise ValueError('Inspected assignment image changed or is missing: '+node)
    native = {r['index']: r for r in json.loads(NATIVE.read_text())['masks']}
    for index in candidate['mask_indices'] + candidate.get('exclude_mask_indices', []):
        if inspected.get(str(index)) != sha(NATIVE.parent/native[index]['png']):
            raise ValueError('Inspected native bitmap changed or is unbound: '+str(index))
    authored = candidate.get('authored_mask')
    if authored:
        if sha(authored['path']) != authored['sha256']:
            raise ValueError('Authored mask changed: '+node)
        if sha(authored['card']) != authored['card_sha256']:
            raise ValueError('Authored mask inspection changed: '+node)
        if authored.get('visually_inspected') is not True:
            raise ValueError('Authored mask has not been inspected: '+node)
    return {**candidate, 'ownership_status': 'reviewed'}


def verify_coverage(constraints, objects):
    missing = [o['name'] for o in objects if constraints.for_object(o) is None]
    if missing:
        raise ValueError('Unconstrained source receivers: '+', '.join(missing))
    return len(objects)


def main(recipe_path, grouping, output, models=None):
    recipe_path, grouping, output = map(lambda p: Path(p).resolve(), (recipe_path, grouping, output))
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(recipe_path, output/'recipe.json')
    recipe_path = output/'recipe.json'
    recipe = json.loads(recipe_path.read_text())
    worker = grouping/'grouped-source-only.blend'
    model_evidence = {}
    if models:
        models = Path(models).resolve()
        model = json.loads(models.read_text())
        if model['status'] != 'MODELS_APPROVED' or sha(worker) != model['grouped_worker_sha256']:
            raise ValueError('Model approval does not match grouped baseline')
        worker = Path(model['worker'])
        if sha(worker) != model['worker_sha256']:
            raise ValueError('Approved model worker changed')
        for path, expected in model['evidence'].items():
            if sha(path) != expected:
                raise ValueError('Approved model evidence changed: '+path)
        model_evidence[str(models)] = sha(models)
        model_evidence.update(model['evidence'])
    approval = json.loads((grouping/'grouping-approval.json').read_text())
    if approval['status'] != 'GROUPING_APPROVED':
        raise ValueError('Grouping is not approved')
    geometry = json.loads((grouping/'geometry.json').read_text())
    plan = json.loads((grouping/'plan.json').read_text())
    inventory = json.loads(NATIVE.read_text())
    for record in inventory['masks']:
        record['png'] = str((NATIVE.parent/record['png']).resolve())
    Image.new('L', (1, 1), 0).save(output/'unresolved.png')
    inventory['masks'].append(dict(index=10000, box_top_left=[0, 0], box_size=[1, 1], png='unresolved.png',
                                   purpose='Explicitly unknown source ownership; never unconstrained'))
    write(output/'inventory.json', inventory)
    by_node = {r['source_node']: r for r in recipe['assignments']}
    if len(by_node) != len(recipe['assignments']):
        raise ValueError('Duplicate recipe source node')
    nodes = sorted({r['source'] for r in geometry})
    assignments = []
    for node in nodes:
        candidate = by_node.get(node, {'source_node': node})
        reason = recipe['unresolved'].get(node)
        if node == 'ground' and candidate.get('reviewed') is not True:
            reason = 'Ground requires a separate terrain-domain and foreground-exclusion audit'
        if node == 'foliage-foreground-oak' and candidate.get('reviewed') is not True:
            reason = 'Canopy is a separate original animation layer, not Day artwork'
        assignments.append(effective_rule(candidate, reason))
    seen_components=set()
    for candidate in recipe.get('component_assignments',[]):
        key=(candidate['source_node'],candidate['projection_component'])
        if key in seen_components:raise ValueError('Duplicate component mask: '+str(key))
        seen_components.add(key)
        if not any(r['source']==key[0] and r['name']==key[1] for r in geometry):
            raise ValueError('Component does not exist in approved models: '+str(key))
        rule=effective_rule(candidate)
        rule['projection_component']=candidate['projection_component']
        assignments.append(rule)
    for index, rule in enumerate(assignments, 20000):
        if rule['ownership_status'] != 'reviewed' or not rule.get('authored_mask'):
            continue
        authored = rule['authored_mask']
        image = Image.open(authored['path'])
        if image.size != Image.open(SOURCE).size:
            raise ValueError('Authored source domain must use map coordinates')
        filename = f'authored-{index}.png'
        shutil.copy2(authored['path'], output/filename)
        inventory['masks'].append(dict(index=index,box_top_left=[0,0],box_size=list(image.size),png=filename))
        rule['native_mask_indices'] = rule['mask_indices']
        rule['mask_indices'] = [index]
        rule.pop('exclude_mask_indices',None)
    write(output/'inventory.json', inventory)
    manifest = dict(version=1, mask_inventory='inventory.json', projections={
        'exterior': dict(source_sha256=sha(SOURCE), state=recipe['state'], assignments=assignments)})
    write(output/'source-masks.json', manifest)
    constraints = SourceMaskConstraints(output/'source-masks.json', 'exterior', sha(SOURCE),
                                        Image.open(SOURCE).size, image_loader=load_image)
    receivers = [dict(name=r['name'], source_node=r['source'], asset_group=plan['assignments'][r['name']],
                      projection_component=r['name'] if r['source'] in {'building-024', 'building-102'} else None)
                 for r in geometry]
    verify_coverage(constraints, receivers)
    unknown = {r['source_node']: r['reason'] for r in assignments if r['ownership_status'] == 'unresolved'}
    counts = {node: sum(r['source'] == node for r in geometry) for node in nodes}
    report = dict(status='PARTIAL_AUDIT' if unknown else 'DAY_MASK_ASSIGNMENTS_COMPLETE', synthesis_ready=False,
        scope='Day source assignments; separate canopy-layer validation is required before synthesis',
        source_sha256=sha(SOURCE), recipe_sha256=sha(recipe_path), grouping_approval_sha256=sha(grouping/'grouping-approval.json'),
        grouped_worker_sha256=sha(grouping/'grouped-source-only.blend'), worker=str(worker),
        worker_sha256=sha(worker), constrained_receivers=len(receivers),
        unconstrained_receivers=0, reviewed_source_nodes=len(nodes)-len(unknown),
        unresolved_source_nodes=len(unknown), unresolved_meshes=sum(counts[n] for n in unknown),
        unresolved=unknown, evidence=evidence_record(output/'source-masks.json'))
    report['evidence'].update(model_evidence)
    report['evidence'][str(recipe_path)] = sha(recipe_path)
    for rule in assignments:
        if rule['ownership_status'] == 'reviewed':
            card = (EDITOR/rule['inspection']['card']).resolve()
            report['evidence'][str(card)] = sha(card)
            if rule.get('authored_mask'):
                for key in ('path','card'):
                    path=rule['authored_mask'][key]
                    report['evidence'][str(path)]=sha(path)
    write(output/'coverage.json', report)
    print(json.dumps({k:v for k,v in report.items() if k not in {'unresolved', 'evidence'}}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--recipe', default=DEFAULT_RECIPE)
    parser.add_argument('--grouping', default=DEFAULT_GROUPING)
    parser.add_argument('--output', required=True)
    parser.add_argument('--models', help='Approval receipt for a revised model baseline')
    args = parser.parse_args()
    main(args.recipe, args.grouping, args.output, args.models)
