"""Pin the separately reviewed root addition before a private crown rebuild."""
import json
from pathlib import Path
from evidence_io import sha


def validate(root_worker, approved_worker):
    if root_worker.name != 'croisement02-tree-15' or approved_worker.name != root_worker.name:
        raise ValueError('Root/crown combination is scoped to tree15')
    model_hash = sha(root_worker / 'model.blend')
    proof = json.loads((root_worker / 'inspection/root-preservation.json').read_text())
    review = json.loads((root_worker / 'inspection/root-component-review.json').read_text())
    completion = json.loads((root_worker / 'inspection/root-completion.json').read_text())
    audit = json.loads((root_worker / 'inspection/saved-model-audit.json').read_text())
    coverage = json.loads((root_worker / 'inspection/root-source-coverage/report.json').read_text())
    if any(record['model_sha256'] != model_hash for record in (proof, review, completion, audit, coverage)):
        raise ValueError('Stale root component evidence')
    if (Path(proof['previous_worker']) != approved_worker or
            proof['previous_model_sha256'] != sha(approved_worker / 'model.blend') or
            not proof['preserved'] or proof['previous_meshes'] != proof['current_meshes']):
        raise ValueError('Root component no longer preserves the approved base')
    if not review['root_component_ready'] or audit['status'] != 'PASS' or coverage['source_coverage'] < .95:
        raise ValueError('Root component failed local review')
    if sha(Path(completion['bank_worker']) / 'model.blend') != completion['bank_model_sha256']:
        raise ValueError('Joint bank geometry changed')
    joint = Path(review['joint_evidence'])
    if sha(joint) != review['joint_evidence_sha256']:
        raise ValueError('Root joint review changed')
    paths = [root_worker / name for name in ['model.blend', 'source-masks.json',
        'inspection/root-preservation.json', 'inspection/root-component-review.json',
        'inspection/root-completion.json', 'inspection/saved-model-audit.json',
        'inspection/root-source-coverage/report.json']]
    manifest = json.loads((root_worker / 'source-masks.json').read_text())
    inventory = Path(manifest['mask_inventory'])
    paths.extend([inventory, joint, Path(completion['bank_worker']) / 'model.blend'])
    paths.extend(Path(row['png']) for row in json.loads(inventory.read_text())['masks'])
    return dict(worker=str(root_worker), model_sha256=model_hash, root_component_reviewed=True,
                evidence_sha256={str(path): sha(path) for path in paths})
