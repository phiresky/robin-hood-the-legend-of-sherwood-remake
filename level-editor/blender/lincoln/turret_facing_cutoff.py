"""Property-only source-projection facing cutoff for lincoln-southeast-corner-turret.

The approved turret has large near-grazing faces (source-facing cosine 0.09 and 0.14 on the
body, 0.09-0.13 on the parapet). Source projection stretches single artwork pixels across
them into long streaks, which then count as protected source texels. Setting
`projection_min_cosine = 0.2` on both meshes (honored by the frozen source_projection_bake and
refinement_review) makes those faces unknown so texture generation can fill them. Every face
at cosine >= 0.23 keeps its source projection.

Run from the repository root:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/turret_facing_cutoff.py -- apply
    /usr/bin/blender --background <workspace>/model.blend --threads 2 --python-exit-code 1 \
      --python <tooling>/refinement_workspace.py -- modified <workspace>
    python3 level-editor/blender/lincoln/turret_facing_cutoff.py report

`apply` backs up model.blend, modified/ and candidate.json to
projection-correction-reference/<old model sha12>/, changes only the two object properties,
and proves mesh geometry and transforms are byte-identical. `report` writes
projection-correction.json (status PASS only after all eight modified views were inspected,
recorded with --inspected) and binds candidate.json to the new model and packet.
"""
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
WORKSPACE = WORK / 'round-3/assets/lincoln-southeast-corner-turret'
ASSET = 'lincoln-southeast-corner-turret'
CUTOFF = 0.2
NODES = ('building-083', 'building-099')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def geometry():
    import bpy
    records = {o.name: {'vertices': [list(v.co) for v in o.data.vertices],
                        'faces': [list(p.vertices) for p in o.data.polygons],
                        'matrix_world': [list(r) for r in o.matrix_world]}
               for o in bpy.data.objects if o.type == 'MESH'}
    return hashlib.sha256(json.dumps(records, sort_keys=True).encode()).hexdigest()


def apply():
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from render_slots import acquire
    acquire()
    import bpy
    model = WORKSPACE / 'model.blend'
    approval = [a for a in json.loads((WORK / 'approvals.json').read_text())['approvals'] if a['asset_id'] == ASSET]
    if len(approval) != 1 or approval[0]['decision'] != 'approved' or approval[0]['model_sha256'] != sha(model):
        raise ValueError('Current model is not the approved turret revision')
    before_hash = sha(model)
    backup = WORKSPACE / 'projection-correction-reference' / before_hash[:12]
    if backup.exists():
        raise FileExistsError(backup)
    backup.mkdir(parents=True)
    shutil.copy2(model, backup / 'model.blend')
    shutil.copytree(WORKSPACE / 'modified', backup / 'modified')
    shutil.copy2(WORKSPACE / 'candidate.json', backup / 'candidate.json')
    shutil.copy2(WORKSPACE / 'source-coverage-audit.json', backup / 'source-coverage-audit.json')
    bpy.ops.wm.open_mainfile(filepath=str(model))
    geometry_before = geometry()
    changes = []
    for obj in bpy.data.collections['lincoln Working'].all_objects:
        if obj.type == 'MESH' and obj.get('asset_group') == ASSET:
            if obj.get('source_node') not in NODES or obj.get('projection_min_cosine') is not None:
                raise ValueError('Unexpected turret mesh or existing cutoff: ' + obj.name)
            obj['projection_min_cosine'] = CUTOFF
            changes.append({'object': obj.name, 'source_node': obj.get('source_node'),
                            'property': 'projection_min_cosine', 'before': None, 'after': CUTOFF})
    if sorted(c['source_node'] for c in changes) != sorted(NODES):
        raise ValueError('Turret meshes differ from the expected source nodes')
    geometry_after = geometry()
    if geometry_after != geometry_before:
        raise RuntimeError('Property change altered geometry')
    bpy.ops.wm.save_as_mainfile(filepath=str(model))
    state = {'approved_model_sha256': before_hash, 'geometry_before_sha256': geometry_before,
             'geometry_after_sha256': geometry_after, 'object_properties': changes,
             'backup': str(backup)}
    (WORKSPACE / 'projection-correction-state.json').write_text(json.dumps(state, indent=2) + '\n')
    print(json.dumps(state), flush=True)


def report(inspected, observation):
    state = json.loads((WORKSPACE / 'projection-correction-state.json').read_text())
    model_hash = sha(WORKSPACE / 'model.blend')
    views_hash = sha(WORKSPACE / 'modified/views.json')
    correction = {
        'version': 1, 'asset_id': ASSET, 'kind': 'property-only source-projection facing cutoff',
        'status': 'PASS' if inspected else 'pending-inspection',
        'approved_model_sha256': state['approved_model_sha256'], 'model_sha256': model_hash,
        'modified_views_sha256': views_hash,
        'geometry_before_sha256': state['geometry_before_sha256'],
        'geometry_after_sha256': state['geometry_after_sha256'],
        'object_properties': state['object_properties'],
        'reason': ('Near-grazing faces (source-facing cosine 0.09-0.14) stretched single artwork pixels '
                   'into long streaks that were protected as source texels; they become unknown for generation.'),
        'inspected_views': list(range(8)) if inspected else [],
        'observation': observation, 'backup': state['backup'],
        'recipe': str(Path(__file__).resolve())}
    path = WORKSPACE / 'projection-correction.json'
    path.write_text(json.dumps(correction, indent=2) + '\n')
    candidate_path = WORKSPACE / 'candidate.json'
    candidate = json.loads(candidate_path.read_text())
    candidate.update(model_sha256=model_hash, modified_views_sha256=views_hash,
                     projection_correction='projection-correction.json')
    note = 'Projection facing cutoff 0.2 on both meshes (property-only; geometry byte-identical).'
    if note not in candidate['changes']:
        candidate['changes'].append(note)
    candidate_path.write_text(json.dumps(candidate, indent=2) + '\n')
    audit_path = WORKSPACE / 'source-coverage-audit.json'
    audit = json.loads(audit_path.read_text())
    audit.update(model_sha256=model_hash, modified_views_sha256=views_hash,
                 inspected_views=list(range(8)) if inspected else [],
                 status='PASS' if inspected else 'FAIL',
                 projection_correction=('Faces with source-facing cosine below 0.2 are deliberately unknown '
                                        '(grazing streaks); ' + observation))
    audit_path.write_text(json.dumps(audit, indent=2) + '\n')
    print(json.dumps({'correction': str(path), 'status': correction['status']}), flush=True)


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else sys.argv[1:]
    if args[0] == 'apply':
        apply()
    elif args[0] == 'report':
        report('--inspected' in args, args[args.index('--observation') + 1] if '--observation' in args else '')
    else:
        raise SystemExit('Expected apply or report [--inspected --observation TEXT]')
