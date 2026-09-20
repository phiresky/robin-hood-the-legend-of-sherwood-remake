"""Build focused worker evidence without altering frozen projection references."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from PIL import Image

from interior_layers import projection_receivers


def prepare(workspace):
    workspace = Path(workspace).resolve()
    config = json.loads((workspace / 'workspace.json').read_text())
    views = json.loads((workspace / 'input/views.json').read_text())
    reference = workspace / 'reference'
    layers = json.loads((reference / 'layers.json').read_text())
    owned = set(config['part_ids'])
    roles = projection_receivers(layers)
    selected = {patch for patch, nodes in roles.items() if owned.intersection(nodes)}
    bounds = views['context_crop']
    box = tuple(bounds[k] for k in ('left', 'top', 'right', 'bottom'))
    output = workspace / 'asset-reference'
    output.mkdir(exist_ok=False)
    evidence = []

    def crop(source, name, role):
        source = Path(source).resolve(strict=True)
        with Image.open(source) as original:
            if not (0 <= box[0] < box[2] <= original.width and
                    0 <= box[1] < box[3] <= original.height):
                raise ValueError(f'Invalid source bounds for {source}')
            original.crop(box).save(output / name)
        evidence.append(dict(file=name, role=role, source=str(source),
                             source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                             source_crop=list(box)))

    crop(views['source_image'], 'source-context.png', 'source state used for exterior review')
    crop(reference / layers['sources']['exterior'], 'covered.png', 'covered exterior')
    if selected:
        crop(reference / layers['sources']['interior'], 'revealed.png', 'revealed interior')
    patches = []
    for patch in layers['patches']:
        if patch['id'] not in selected:
            continue
        graphic = patch.get('graphic')
        if not graphic:
            raise ValueError(f'Missing graphic for reviewed interior {patch["id"]}')
        directory = output / patch['id']
        directory.mkdir()
        for kind in ('image', 'alpha'):
            source = reference / graphic[kind]
            destination = directory / f'{kind}.png'
            shutil.copy2(source, destination)
        patches.append(dict(id=patch['id'], name=patch['name'],
                            bbox=graphic['bbox'],
                            owned_interior_nodes=sorted(owned.intersection(roles[patch['id']])),
                            image=f'{patch["id"]}/image.png',
                            alpha=f'{patch["id"]}/alpha.png'))
    # Mission state membership is separate from authored interior receiver roles.
    # Preserve explicit cross-asset state dependencies as links, not ownership.
    mission_states = []
    for patch in layers.get('mission_patches', []):
        affected = owned.intersection(patch.get('sight_before', []) + patch.get('sight_after', []))
        if affected:
            mission_states.append(dict(id=patch['id'], name=patch['name'],
                                       affected_owned_nodes=sorted(affected),
                                       relation='sight-state dependency; not inferred visual ownership',
                                       backing_manifest=str(reference / 'layers.json')))
    report = dict(asset_id=config['asset_id'], source_origin=list(box[:2]),
                  evidence=evidence, interior_patches=patches,
                  mission_state_dependencies=mission_states,
                  backing_reference=str(reference),
                  limitations=['A crop can include neighboring context; pixels are not an ownership mask.',
                               'Interior patch selection uses reviewed receiver assignments, not bounding-box overlap.',
                               'Mission visual dependencies still require review of the backing manifest.'])
    (output / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    (output / 'README.md').write_text(
        f'# Focused reference: {config["asset_id"]}\n\n'
        'Start with source-context.png and covered.png. For interior assets, compare\n'
        'revealed.png and the listed patch image/alpha pairs. Crops keep original\n'
        'pixels and share the same map-space origin in manifest.json.\n\n'
        'Only patches with reviewed interior receivers belonging to this asset\n'
        'are included. Neighboring artwork in a crop is context, not owned geometry.\n'
        'The full reference/ folder is projection backing data, not a list of\n'
        'assets or patches assigned to this worker. Consult its layers.json for\n'
        'mission-state dependencies and any additional occlusion investigation.\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workspace', type=Path)
    args = parser.parse_args()
    print(json.dumps(prepare(args.workspace), indent=2))
