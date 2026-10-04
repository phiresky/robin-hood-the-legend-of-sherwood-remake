"""Strict selection for reviewed tree07 lower-wood revisions; crown stays frozen."""
import argparse
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json
from catalog_schema import source_for_part


def read(path):
    return json.loads(path.read_text())


def require(value, message):
    if not value:
        raise ValueError(message)


def validate_worker(worker):
    require(worker.name == 'croisement02-tree-07', 'Wood revision scope is tree07 only')
    digest = sha(worker / 'model.blend')
    inspect = worker / 'inspection'
    review = read(inspect / 'visual-review.json')
    require(review.get('ready_for_user'), 'Wood revision needs independent final review')
    independent = read(inspect / 'independent-wood-review.json')
    require(independent['model_sha256'] == digest and independent['status'].startswith('PASS')
            and any('root independent' in reviewer for reviewer in independent['reviewers']),
            'Missing independent scoped wood review')
    for path, expected in independent['files'].items():
        require(sha(Path(path)) == expected, 'Independent reviewed image changed: ' + path)
    proof = read(inspect / 'root-preservation.json')
    coverage = read(inspect / 'source-coverage/report.json')
    local = read(inspect / 'root-source-coverage/report.json')
    audit = read(inspect / 'saved-model-audit.json')
    bounds = read(inspect / 'actual-materials/opacity-bounds.json')
    for row in (review, proof, coverage, local, audit, bounds):
        require(row['model_sha256'] == digest, 'Stale wood revision evidence')
    require(read(worker / 'validation.json')['status'] == 'PASS' and audit['status'] == 'PASS',
            'Wood revision validation failed')
    expected = {'North Tree 07 / Crown', 'North Tree 07 / North Tree 07 wood 060',
                'North Tree 07 / North Tree 07 wood 061'}
    require(proof['preserved'] and proof['previous_meshes'] == proof['current_meshes']
            and set(proof['current_meshes']) == expected, 'Crown or protected upper wood changed')
    previous = Path(proof['previous_worker'])
    require(previous != worker and sha(previous / 'model.blend') == proof['previous_model_sha256'],
            'Independent approved base changed')
    require(coverage['intersection_over_union'] >= .95 and all(local[k] >= .95 for k in
            ('source_coverage', 'interface_source_coverage', 'root_source_coverage')),
            'Wood source coverage failed')
    require(bounds['crowns'] and min(c['depth_width_ratio'] for c in bounds['crowns']) >= 1,
            'Preserved crown lost full depth')
    junction = read(inspect / 'refinement.json')['lower_stem']['upper_junction']
    require(junction['upper_vertex_positions_identical'] and junction['retained_upper_vertices'] == 10
            and junction['shared_boundary_normals'] == 36, 'Upper062 junction preservation failed')
    joint = read(inspect / 'joint-neighbourhood.json')
    require(joint['model_sha256'] == digest and sha(Path(joint['evidence'])) == joint['evidence_sha256']
            and sha(Path(joint['sheet'])) == joint['sheet_sha256'], 'Joint contact proof changed')
    for row in read(Path(joint['evidence']))['workers']:
        require(sha(Path(row['path']) / 'model.blend') == row['model_sha256'], 'Joint neighbour changed')
    return proof, joint


def selected_workspace(out, mask, catalog_path):
    receipt = out / 'wood-revision-selections' / f'tree-{mask:02}.json'
    if not receipt.exists():
        return None
    record = read(receipt)
    require(mask == 7 and record['kind'] == 'tree07-lower-wood' and record['approval'] == 'pending',
            'Invalid scoped wood selection')
    worker = Path(record['worker'])
    require(record['asset_id'] == worker.name == 'croisement02-tree-07', 'Wrong wood revision asset')
    group = next(g for g in read(catalog_path)['groups'] if g['id'] == worker.name)
    require({source_for_part(p) for p in group['parts']} == set(record['part_ids']),
            'Wood revision source scope changed')
    for path, expected in record['evidence_sha256'].items():
        require(sha(Path(path)) == expected, 'Wood revision evidence changed: ' + path)
    require(sha(worker / 'model.blend') == record['model_sha256'], 'Wood revision worker changed')
    validate_worker(worker)
    return worker


def expose(worker):
    from catalog import OUT, reviewed_catalog
    proof, joint = validate_worker(worker)
    cfg = read(worker / 'workspace.json')
    decisions = {row['asset_id']: row for row in read(OUT / 'user-feedback.json')['records']}
    previous_approval = decisions[worker.name]
    require(previous_approval['decision'] == 'approved'
            and previous_approval['model_sha256'] == proof['previous_model_sha256'],
            'Lower wood revision requires the current approved crown base')
    receipt = OUT / 'wood-revision-selections/tree-07.json'
    require(not receipt.exists(), 'Preserve the existing wood revision receipt')
    paths = {worker / name for name in ('model.blend', 'baseline.blend', 'workspace.json',
                                       'validation.json', 'source-masks.json')}
    for directory in ('inspection', 'recipe', 'reference', 'mask-reference', 'modified'):
        paths.update(p for p in (worker / directory).rglob('*') if p.is_file())
    paths.add(Path(proof['previous_worker']) / 'model.blend')
    archive = Path(previous_approval['archive'])
    paths.update(p for p in archive.rglob('*') if p.is_file())
    joint_path = Path(joint['evidence'])
    paths.update(p for p in joint_path.parent.rglob('*') if p.is_file())
    for row in read(joint_path)['workers']:
        paths.add(Path(row['path']) / 'model.blend')
    for path, expected in read(worker / 'mask-reference/native-hashes.json').items():
        require(sha(Path(path)) == expected, 'Native mask changed')
        paths.add(Path(path))
    receipt.parent.mkdir(exist_ok=True)
    write_json(receipt, dict(kind='tree07-lower-wood', asset_id=worker.name, worker=str(worker),
        model_sha256=sha(worker / 'model.blend'), part_ids=cfg['part_ids'], approval='pending',
        previous_worker=proof['previous_worker'], previous_model_sha256=proof['previous_model_sha256'],
        previous_geometry_approval=previous_approval,
        evidence_sha256={str(p): sha(p) for p in sorted(paths)},
        scope='058 and lower062 repaired; crown/060/061 unchanged and upper062 positions retained. New geometry approval required.'))
    require(selected_workspace(OUT, 7, reviewed_catalog()) == worker, 'Selection failed')
    print(receipt)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    expose(parser.parse_args().worker.resolve())
