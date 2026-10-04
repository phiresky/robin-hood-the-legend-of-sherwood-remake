"""Complete the cropped northern tree without claiming its neighbours' foliage."""
import json
import sys
import uuid
from pathlib import Path
import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, tree_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import prepare, modified, validate
from complete_north_tree23 import silhouette_wood, inferred_crown
from tree_geometry import replace_mesh, RAY
from bark_materials import fill
from audit_candidates import audit
from render_tree import render_workspace


def overlap_evidence():
    layers, inputs = {}, []
    rows = json.loads((OUT / 'forest-v4-sources/manifest.json').read_text())
    for row in rows:
        worker = tree_workspace(row['mask'])
        report = json.loads((worker / 'inspection/refinement.json').read_text())
        packet_path = Path(report.get('source_packet', row['packet']))
        packet = json.loads(packet_path.read_text())
        path = packet_path.parent / 'complete-source.png'
        x, y, width, height = packet['native_bbox']
        alpha = np.asarray(Image.open(path).convert('RGBA'))[:, :, 3] > 127
        layer = np.zeros((1152, 1792), bool)
        left, top, right, bottom = max(0, x), max(0, y), min(1792, x + width), min(1152, y + height)
        layer[top:bottom, left:right] = alpha[top-y:bottom-y, left-x:right-x]
        layers[row['mask']] = layer
        inputs.append(dict(mask=row['mask'], image=str(path), sha256=sha(path)))
    other = np.logical_or.reduce([v for k, v in layers.items() if k != 20])
    unique = int(np.count_nonzero(layers[20] & ~other))
    if unique:
        raise ValueError('Removing this foliage assignment would leave an unowned native region')
    return dict(former_pixels=int(layers[20].sum()), uniquely_owned_pixels=unique, inputs=inputs,
                limitation='Compares source-domain ownership; final integrated geometry coverage is a separate check.')


def main():
    asset = 'croisement02-tree-20'
    old = OUT / 'forest-v4-round-1/assets' / asset
    worker = OUT / 'forest-v4-round-2/assets' / asset
    directory = OUT / 'tree20-source-revision'
    receipt = worker / 'inspection/source-domain-revision.json'
    latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    if latest.get(asset, {}).get('decision') == 'approved':
        raise ValueError('Approved geometry is frozen')
    acquire()
    try:
        if receipt.exists() and '--redo' in sys.argv:
            receipt.rename(receipt.with_name('source-domain-revision-archive-' + uuid.uuid4().hex[:8] + '.json'))
        if not (worker / 'workspace.json').exists():
            overlap = overlap_evidence()
            directory.mkdir(exist_ok=True)
            write_json(directory / 'canopy-overlap.json', overlap)
            source = Image.open(OUT / 'animation-references/composite-frame-0.png').convert('RGBA')
            inventory = json.loads((OUT / 'baseline/masks/manifest.json').read_text())['masks']
            leaf = next(r for r in inventory if r['index'] == 135)
            x, y = leaf['box_top_left']
            width, height = leaf['box_size']
            palette = source.crop((x, y, x + width, y + height))
            palette.putalpha(Image.open(OUT / 'baseline/masks' / leaf['png']).convert('L'))
            palette.save(directory / 'native-northeast-leaves.png')
            bpy.ops.wm.open_mainfile(filepath=str(old / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            crown = next(o for o in objects if o.get('projection_component') == 'crown')
            crown_report = inferred_crown(crown, center_x=1704, ground_y=87.191774,
                                         palette=directory / 'native-northeast-leaves.png')
            native = next(r for r in inventory if r['index'] == 20)
            alpha = np.asarray(Image.open(OUT / 'baseline/masks' / native['png']).convert('L')) > 0
            yy, xx = np.indices(alpha.shape)
            root = alpha & (xx + native['box_top_left'][0] >= 1708) & (yy >= 70)
            parts = []
            for obj in [o for o in objects if o.get('projection_component') != 'crown']:
                index = int(obj['source_node'].split('-')[-1])
                domain = root if index == 107 else alpha & ~root
                vertices, faces = silhouette_wood(20, 87.191774, center_x=1704, domain=domain)
                points = np.asarray(vertices)
                z = points[:, 2].copy()
                height = .5 * (z + np.sqrt(z * z + 36))
                points += (height - z)[:, None] * np.asarray(RAY)[None, :] / RAY.z
                result = replace_mesh(obj, points.tolist(), faces, materials=list(obj.data.materials))
                result['source_node'] = obj['source_node']
                for face in obj.data.polygons:
                    face.use_smooth = len(face.vertices) == 4
                parts.append(result)
            prepare(worker, asset_id=asset, scene_name='Croisement02 Refinement',
                collection_name='Croisement02 Working', source_path=OUT / 'animation-references/composite-frame-0.png',
                grouping_manifest=OUT / 'catalog.json', inventory_path=OUT / 'forest-v4-inventory/inventory.json',
                review_path=OUT / 'forest-v4-grouping-review.json', source_mask_manifest=old / 'source-masks.json',
                width=384, height=384, framing_padding=1.4,
                lighting=dict(toward_sun=[-.45, -.55, .70], ambient=.22, diffuse=.78, shadow_epsilon=.05))
            inspection = worker / 'inspection'
            inspection.mkdir(exist_ok=True)
            evidence = inspection / 'off-map-source'
            evidence.mkdir(exist_ok=True)
            Image.new('RGBA', (1792, 1152)).save(evidence / 'complete-source.png')
            write_json(evidence / 'partition.json', dict(native_bbox=[0, 0, 1792, 1152], observed_foliage_pixels=0,
                evidence='Former in-map foliage is fully covered by neighbouring source domains; this tree has an inferred crown above the northern edge.'))
            write_json(inspection / 'refinement.json', dict(asset_id=asset, mask=20, wood=parts, crown=crown_report,
                source_packet=str(evidence / 'partition.json'), status='geometry candidate; visual review pending',
                limitations=['Most native wood is hidden by foreground canopy 135; unseen bark pattern is inferred.',
                             'Crown and upper trunk are completed outside the map. The former in-map foliage remains owned by neighbouring trees.',
                             'Root cross sections, native-part split and hidden ground contact are inferred.']))
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            config = json.loads((worker / 'workspace.json').read_text())
            path = Path(config['source_mask_manifest'])
            masks = json.loads(path.read_text())
            assignment = next(a for a in masks['projections']['exterior']['assignments'] if a.get('asset_group') == asset)
            assignment.update(exclude_mask_indices=[135], exclusions_reviewed=True,
                exclusion_reason='Inspected wood/canopy overlay: only 128 native wood pixels are outside foreground canopy 135. Leaves must not become bark.')
            write_json(path, masks)
            write_json(worker / 'inspection/bark-donor-selection.json', dict(native_mask=20, source_box=[1710, 54, 1713, 57],
                source_sha256=sha(worker / 'reference/source.png'), reviewer='Codex',
                notes='Small exposed dark wood sample outside canopy 135. Its palette supports inferred smooth grain only; it does not establish an observed rear bark pattern.'))
            crown = next(o for o in objects if o.get('projection_component') == 'crown')
            crown_report = inferred_crown(crown, center_x=1704, ground_y=87.191774,
                                         palette=directory / 'native-northeast-leaves.png')
            modified(worker)
            bark = fill(worker, objects, 20, receiver_only=True, donor_mapping='continuous-grain')
            validate(worker)
            bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
            report = json.loads((worker / 'inspection/refinement.json').read_text())
            report.update(model_sha256=sha(worker / 'model.blend'), bark=bark, crown=crown_report)
            write_json(worker / 'inspection/refinement.json', report)
            audit(worker)
            write_json(receipt, dict(model_sha256=report['model_sha256'], previous_model_sha256=sha(old / 'model.blend'),
                overlap_evidence_sha256=sha(directory / 'canopy-overlap.json'), classification='Northern tree with inferred off-map crown'))
        elif json.loads(receipt.read_text())['model_sha256'] != sha(worker / 'model.blend'):
            raise ValueError('Northern tree changed')
        render_workspace(worker, 384, release_slot=False)
    finally:
        release()


if __name__ == '__main__':
    main()
