"""Reconcile selected foliage appearance evidence without manufacturing approvals."""
import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog, scenery_workspace
from evidence_io import sha, write_json


def run(destination):
    catalog_path = reviewed_catalog()
    catalog_hash = sha(catalog_path)
    catalog = json.loads(catalog_path.read_text())
    rows = []
    for group in catalog['groups']:
        if not ({'native_foliage_mask', 'native_foliage_masks'} & group.keys()):
            continue
        worker = scenery_workspace(group['id'])
        model_hash = sha(worker / 'model.blend')
        inspection = worker / 'inspection'
        errors = []
        bindings = {}
        proofs = {}
        for name in ['saved-model-audit.json', 'visual-review.json',
                     'actual-materials/evidence.json', 'source-coverage/report.json']:
            path = inspection / name
            if not path.exists():
                errors.append('Missing ' + name)
                continue
            bindings[str(path)] = sha(path)
            proof = json.loads(path.read_text())
            proofs[name] = proof
            if proof.get('model_sha256') != model_hash:
                errors.append('Stale model binding: ' + name)
        review = proofs.get('visual-review.json', {})
        sheet = inspection / 'actual-materials/sheet.png'
        bindings[str(sheet)] = sha(sheet)
        if review.get('sheet_sha256') != bindings[str(sheet)]:
            errors.append('Actual appearance sheet changed since review')
        if review.get('preservation_evidence'):
            preservation = Path(review['preservation_evidence'])
            bindings[str(preservation)] = sha(preservation)
            if review.get('preservation_evidence_sha256') != bindings[str(preservation)]:
                errors.append('Observed RGB preservation evidence changed')
            preserved = json.loads(preservation.read_text())
            package_preserved=(preserved.get('exact_geometry_uv_material_preservation') is True
                               and preserved.get('model_sha256')==model_hash
                               and bool(preserved.get('protected'))
                               and all(sha(Path(p))==digest for p,digest in preserved['protected'].items()))
            if preserved.get('status') != 'PASS' and not package_preserved:
                errors.append('Observed RGB preservation is not PASS')
        else:
            preservation=inspection/'package-preservation.json'
            if preservation.exists():
                preserved=json.loads(preservation.read_text());bindings[str(preservation)]=sha(preservation)
                if not (preserved.get('exact_geometry_uv_material_preservation') is True
                        and preserved.get('model_sha256')==model_hash
                        and bool(preserved.get('protected'))
                        and all(sha(Path(p))==digest for p,digest in preserved['protected'].items())):
                    errors.append('Exact package appearance preservation failed')
            else:
                errors.append('No explicit observed RGB or exact package appearance preservation receipt')
        images = []
        for obj in proofs.get('saved-model-audit.json', {}).get('objects', []):
            for material in obj['used_materials']:
                inferred = 'inferred' in material['name'].lower()
                if inferred and not material.get('images'):
                    errors.append('Untextured inferred material: ' + material['name'])
                for image in material.get('images', []):
                    if not image.get('packed_sha256'):
                        errors.append('Unbound image in ' + material['name'])
                    images.append(dict(object=obj['object'], material=material['name'],
                                       inferred=inferred, **image))
        rows.append(dict(asset_id=group['id'], worker=str(worker), model_sha256=model_hash,
                         status='PASS' if not errors else 'HOLD', errors=errors,
                         appearance='Own-native inferred leaf textures; no API required by recipe',
                         images=images, evidence=bindings,
                         user_approval='Not assessed by this audit'))
    wanted = {image['packed_sha256'] for row in rows for image in row['images']
              if image.get('packed_sha256')}
    image_names = {re.sub(r'\.\d+$', '', image['name'])
                   for row in rows for image in row['images']}
    image_files = {}
    for folder in [*OUT.glob('understory*'),OUT/'restart2-vegetation',OUT/'restart2-fence']:
        for path in folder.rglob('*.png'):
            if path.name not in image_names:
                continue
            digest = sha(path)
            if digest in wanted:
                image_files.setdefault(digest, []).append(str(path))
    for row in rows:
        for image in row['images']:
            if image.get('packed_sha256') not in image_files:
                row['errors'].append('No byte-identical recipe PNG found: ' + image['name'])
        row['status'] = 'HOLD' if row['errors'] else 'PASS'
    if sha(catalog_path) != catalog_hash:
        raise ValueError('Catalog changed during audit; rerun against current selection')
    report = dict(status='PASS' if all(r['status'] == 'PASS' for r in rows) else 'HOLD',
                  catalog_sha256=catalog_hash, registered_foliage_groups=len(rows), records=rows,
                  packed_image_recipe_files=image_files,
                  unique_packed_images=len(wanted), matched_recipe_images=len(image_files),
                  limitations=['Evidence reconciliation only; no new visual or user approval.',
                               'Additive packages bind exact geometry/UV/material preservation to their reviewed source candidates; this is not a new native-pixel comparison.',
                               'Packed image binding does not itself prove attractive inferred appearance.',
                               'Ground union, registered group count and source ownership do not prove full scene completion.',
                               'Grass and fern scenery without native_foliage_mask are outside this shrub audit.'])
    write_json(destination, report)
    print(json.dumps(dict(status=report['status'], groups=len(rows),
                          holds=[dict(asset_id=r['asset_id'], errors=r['errors']) for r in rows if r['errors']],
                          output=str(destination)), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args().output.resolve())
