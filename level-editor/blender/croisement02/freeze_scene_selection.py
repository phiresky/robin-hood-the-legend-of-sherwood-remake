"""Freeze strict current worker choices and texture decisions without copying models."""
import argparse
import json
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / 'refinement/blender'))
from catalog import OUT, reviewed_catalog, tree_workspace, scenery_workspace
from evidence_io import sha, write_json
from approved_texture_stage import select


def freeze(base, output):
    if output.exists():
        raise FileExistsError(output)
    catalog_path = reviewed_catalog()
    catalog_hash = sha(catalog_path)
    if sha(base / 'catalog.json') != catalog_hash:
        raise ValueError('Canonical catalog and metadata snapshot differ')
    catalog = json.loads(catalog_path.read_text())
    workers = {g['id']: tree_workspace(g['wood_mask']) if 'wood_mask' in g
               else scenery_workspace(g['id'])
               for g in catalog['groups'] if not g.get('state_only')}
    models = {asset: worker / 'model.blend' for asset, worker in workers.items()}
    model_hashes = {asset: sha(path) for asset, path in models.items()}
    output.mkdir(parents=True)
    metadata = {}
    for name in ('catalog.json', 'source-masks.json', 'mask-inventory.json',
                 'inventory.json', 'grouping-review.json'):
        source = base / name
        shutil.copyfile(source, output / name)
        metadata[name] = dict(source=str(source), sha256=sha(source))
    sources = {}
    mask_inventory = json.loads((base / 'mask-inventory.json').read_text())
    for row in mask_inventory['masks']:
        path = (base / row['png']).resolve()
        sources[str(path)] = sha(path)
    for worker in workers.values():
        frames_path = worker / 'modified/views.json'
        frames = json.loads(frames_path.read_text())
        sources[str(frames_path)] = sha(frames_path)
        image = Path(frames['source_image']).resolve()
        image_hash = sha(image)
        if frames.get('source_sha256') and frames['source_sha256'] != image_hash:
            raise ValueError('Worker source image changed: ' + str(image))
        sources[str(image)] = image_hash
    feedback_path = OUT / 'user-feedback.json'
    shutil.copyfile(feedback_path, output / 'geometry-decisions.json')
    feedback = json.loads(feedback_path.read_text())['records']
    legacy_path = OUT / 'texture-review/decisions.json'
    legacy = json.loads(legacy_path.read_text())
    retained = []
    omitted = []
    for decision in {row['asset_id']: row for row in legacy['decisions']}.values():
        if decision.get('decision') != 'approved' or decision.get('scope') != 'texture':
            retained.append(decision)
            continue
        asset = decision['asset_id']
        proof = Path(decision['evidence_paths']['model']).parent / 'reopened-preservation.json'
        document = json.loads(proof.read_text())
        if document['model_sha256'] == model_hashes.get(asset):
            retained.append(decision)
        else:
            omitted.append(dict(asset_id=asset, decision_model_sha256=decision['evidence_sha256']['model'],
                                old_geometry_sha256=document['model_sha256'],
                                current_geometry_sha256=model_hashes.get(asset),
                                reason='Current geometry differs; do not replace it with an older texture model'))
    filtered = dict(legacy, decisions=retained)
    write_json(output / 'compatible-legacy-texture-decisions.json', filtered)
    selected = select(output / 'compatible-legacy-texture-decisions.json', models)
    canopy_path = OUT / 'canopy-texture-review/decisions.json'
    canopy = select(canopy_path, models)
    if set(selected) & set(canopy):
        raise ValueError('Overlapping independent texture decisions need explicit resolution')
    selected.update(canopy)
    additional_path = OUT / 'texture-review/additional-approved-decisions.json'
    if additional_path.exists():
        additional = select(additional_path, models)
        if set(selected) & set(additional):
            raise ValueError('Additional approved texture stream overlaps historical selections')
        selected.update(additional)
        shutil.copyfile(additional_path, output / 'additional-approved-texture-decisions.json')
    shutil.copyfile(legacy_path, output / 'original-legacy-texture-decisions.json')
    shutil.copyfile(canopy_path, output / 'canopy-texture-decisions.json')
    rows = []
    for group in catalog['groups']:
        asset = group['id']
        if group.get('state_only'):
            rows.append(dict(asset_id=asset, state_only=True, group=group,
                             integration='not included in initial visible scene; state workstreams incomplete'))
            continue
        worker = workers[asset]
        approval = [d for d in feedback if d['asset_id'] == asset
                    and d.get('decision') == 'approved' and d['model_sha256'] == model_hashes[asset]]
        evidence = {}
        for name in ('workspace.json', 'validation.json', 'inspection/saved-model-audit.json',
                     'inspection/visual-review.json', 'inspection/refinement.json',
                     'inspection/source-coverage/report.json', 'inspection/feedback-revision-1.json'):
            path = worker / name
            if path.exists():
                evidence[str(path)] = sha(path)
        texture = selected.get(asset)
        chosen = Path(texture['model']) if texture else models[asset]
        rows.append(dict(asset_id=asset, state_only=False, group=group, worker=str(worker),
                         geometry_model=str(models[asset]), geometry_model_sha256=model_hashes[asset],
                         model=str(chosen), model_sha256=sha(chosen),
                         exact_geometry_user_approval=approval[-1] if approval else None,
                         approved_texture=texture, evidence=evidence))
    if sha(catalog_path) != catalog_hash or any(sha(models[a]) != h for a, h in model_hashes.items()):
        raise ValueError('Selected catalog or worker changed during snapshot')
    if any(sha(Path(row['source'])) != row['sha256'] for row in metadata.values()):
        raise ValueError('Source metadata changed during snapshot')
    if any(sha(Path(path)) != expected for path, expected in sources.items()):
        raise ValueError('Source image, domain or camera changed during snapshot')
    write_json(output / 'selection.json', dict(version=1, status='private immutable selection; not publication',
               catalog_sha256=catalog_hash, metadata=metadata, source_evidence=sources, groups=len(rows),
               visible_groups=len(models), approved_texture_count=len(selected),
               texture_omissions=omitted, records=rows,
               decision_files={name: sha(output / name) for name in
                   ('geometry-decisions.json', 'original-legacy-texture-decisions.json',
                    'compatible-legacy-texture-decisions.json', 'canopy-texture-decisions.json',
                    'additional-approved-texture-decisions.json') if (output / name).exists()},
               model_copies_created=0, complete_state_integration=False))
    print(json.dumps(dict(output=str(output), groups=len(rows), visible_groups=len(models),
                          approved_textures=len(selected), omitted=omitted)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    freeze(args.base.resolve(), args.output.resolve())
