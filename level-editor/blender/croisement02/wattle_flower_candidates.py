"""Select the independently reviewed fence/flower pair with pinned evidence."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json

ASSETS = {
    'croisement02-southwest-path-wattle-fence': 'wattle99-source-candidate/packaged-v6-r2/assets/croisement02-southwest-path-wattle-fence',
    'croisement02-shrub-76': 'restart2-fence/flower76-package-v1/assets/croisement02-shrub-76',
}


def selected_workspace(out, asset, catalog_path):
    if asset not in ASSETS:
        return None
    path = out / 'restart2-fence/pair-selection.json'
    if not path.exists():
        return None
    receipt = json.loads(path.read_text())
    row = receipt['records'][asset]
    group = next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id'] == asset)
    if row['group'] != group:
        raise ValueError('Reviewed fence/flower ownership changed: ' + asset)
    for file, digest in receipt['files'].items():
        if sha(Path(file)) != digest:
            raise ValueError('Reviewed fence/flower evidence changed: ' + file)
    worker = Path(row['workspace'])
    if sha(worker / 'model.blend') != row['model_sha256']:
        raise ValueError('Reviewed fence/flower model changed: ' + asset)
    return worker


def register(out, catalog_path):
    destination = out / 'restart2-fence/pair-selection.json'
    if destination.exists():
        raise FileExistsError(destination)
    catalog_hash = sha(catalog_path)
    catalog = json.loads(catalog_path.read_text())
    pair = out / 'wattle99-source-candidate/v6/flower-joint-completed-v1'
    root = out / 'restart2-fence/root-pair-review.json'
    root_review = json.loads(root.read_text())
    if root_review['status'] != 'PASS scoped pair geometry; user approval pending':
        raise ValueError('Independent pair review is missing')
    first_hit = json.loads((pair / 'first-hit.json').read_text())
    if first_hit != dict(observed_leaf_pixels=1971, physical_leaf_alone_coverage=1971,
                         observed_leaf_blocked_by_fence=0, inferred_boundary_pixels=22,
                         inferred_boundary_covered=22):
        raise ValueError('Reviewed wide-camera pair proof changed')
    files = [root, pair / 'first-hit.json', pair / 'evidence.json', pair / 'self-review.json',
             pair / 'actual-eight.png', pair / 'source-comparison.png', out / 'restart2-fence/pair-binding-v1.json']
    records = {}
    for asset, relative in ASSETS.items():
        worker = out / relative
        digest = sha(worker / 'model.blend')
        group = next(g for g in catalog['groups'] if g['id'] == asset)
        inspection = worker / 'inspection'
        review = json.loads((inspection / 'visual-review.json').read_text())
        audit = json.loads((inspection / 'saved-model-audit.json').read_text())
        coverage = json.loads((inspection / 'source-coverage/report.json').read_text())
        center = json.loads((inspection / 'source-coverage-center-rays/report.json').read_text())
        preservation = json.loads((inspection / 'package-preservation.json').read_text())
        validation = json.loads((worker / 'validation.json').read_text())
        if (root_review['packaged_models'].get(asset) != digest
                or not review['ready_for_geometry_review'] or audit['status'] != 'PASS'
                or validation['status'] != 'PASS'
                or any(r['model_sha256'] != digest for r in [review, audit, coverage, center, preservation])
                or not preservation['exact_geometry_uv_material_preservation']
                or review['sheet_sha256'] != sha(inspection / 'actual-materials/sheet.png')):
            raise ValueError('Packaged pair proof does not bind exact candidate: ' + asset)
        if asset.endswith('76'):
            bounds_path = inspection / 'actual-materials/opacity-bounds.json'
            bounds = json.loads(bounds_path.read_text())
            if (min(coverage['intersection_over_union'], center['intersection_over_union']) < .98
                    or bounds['model_sha256'] != digest or len(bounds['crowns']) != 3
                    or min(r['depth_width_ratio'] for r in bounds['crowns']) < 1):
                raise ValueError('Flower volume or source coverage differs from reviewed bounds')
            files.append(bounds_path)
        else:
            topology_path = inspection / 'fence-topology.json'
            topology = json.loads(topology_path.read_text())
            if (min(coverage['source_recall'], center['source_recall']) < .99
                    or topology['model_sha256'] != digest
                    or any(r['nonmanifold_edges'] or r['degenerate_faces'] for r in topology['objects'])):
                raise ValueError('Wattle topology or native recall changed')
            files.append(topology_path)
        files += [worker / 'model.blend', worker / 'workspace.json', worker / 'source-masks.json', worker / 'validation.json']
        files += [inspection / name for name in ['visual-review.json', 'saved-model-audit.json', 'refinement.json',
                  'package-preservation.json', 'joint-neighbourhood.json', 'source-coverage/report.json',
                  'source-coverage-center-rays/report.json', 'source-coverage-wide-center-rays/report.json',
                  'actual-materials/evidence.json', 'actual-materials/sheet.png']]
        files += [Path(p) for p in preservation['protected']]
        manifest = json.loads((worker / 'source-masks.json').read_text())
        inventory = Path(manifest['mask_inventory'])
        files.append(inventory)
        assigned = {n for r in manifest['projections']['exterior']['assignments']
                    for n in r.get('mask_indices', []) + r.get('exclude_mask_indices', [])}
        files += [(inventory.parent / r['png']).resolve() for r in json.loads(inventory.read_text())['masks'] if r['index'] in assigned]
        records[asset] = dict(workspace=str(worker), model_sha256=digest, group=group,
                             user_approval=False, source_sampling_limit='Wide pair proof and independent tighter crop measurements are retained separately; no universal perfect-coverage claim.')
    if sha(catalog_path) != catalog_hash:
        raise ValueError('Catalog changed during pair registration')
    write_json(destination, dict(status='Independently reviewed replacement geometry candidates; user approval and publication pending',
                                 records=records, files={str(p): sha(p) for p in files},
                                 limitations=['No prior user approval is inherited.',
                                              'Wattle rear fill remains pending.',
                                              'All source-role boundary extensions are separately inferred; native observed domains remain unchanged.']))
    for asset in ASSETS:
        assert selected_workspace(out, asset, catalog_path) == out / ASSETS[asset]
    print(destination)


if __name__ == '__main__':
    from catalog import OUT, reviewed_catalog
    register(OUT, reviewed_catalog())
