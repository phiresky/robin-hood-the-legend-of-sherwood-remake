"""Replace the mislabeled bank proxy with its northern tree and inferred crown."""
import json
import sys
import uuid
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Matrix

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import prepare, modified, validate
from complete_north_tree23 import silhouette_wood, inferred_crown
from tree_geometry import replace_mesh
from bark_materials import fill
from audit_candidates import audit
from render_tree import render_workspace


def main():
    asset = 'croisement02-tree-21'
    worker = OUT / 'forest-v4-round-2/assets' / asset
    directory = OUT / 'tree21-source-revision'
    receipt = worker / 'inspection/source-domain-revision.json'
    latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    if latest.get(asset, {}).get('decision') == 'approved':
        raise ValueError('Approved geometry is frozen')
    acquire()
    try:
        if receipt.exists() and '--redo' in sys.argv:
            receipt.rename(receipt.with_name('source-domain-revision-archive-' + uuid.uuid4().hex[:8] + '.json'))
        if not (worker / 'workspace.json').exists():
            directory.mkdir(exist_ok=True)
            source = Image.open(OUT / 'animation-references/composite-frame-0.png').convert('RGBA')
            native = json.loads((OUT / 'baseline/masks/manifest.json').read_text())['masks']
            leaves = next(r for r in native if r['index'] == 133)
            x, y = leaves['box_top_left']
            width, height = leaves['box_size']
            palette = source.crop((x, y, x + width, y + height))
            palette.putalpha(Image.open(OUT / 'baseline/masks' / leaves['png']).convert('L'))
            palette.save(directory / 'native-northern-leaves.png')
            masks = json.loads((OUT / 'forest-v4-round-1/assets/croisement02-tree-23/source-masks.json').read_text())
            masks['projections']['exterior']['assignments'].extend([
                dict(asset_group=asset, mask_indices=[21], reviewed=True),
                dict(source_node='building-132', projection_component='crown', mask_indices=[133], reviewed=True)])
            write_json(directory / 'assignments.json', masks)
            bpy.ops.wm.open_mainfile(filepath=str(OUT / 'forest-v4-input.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            collection = bpy.data.collections['Croisement02 Working']
            wood = next(o for o in collection.all_objects if o.type == 'MESH' and o.get('source_node') == 'building-132')
            wood['asset_group'] = asset
            wood['asset_name'] = 'Northern Boundary Tree 21'
            ground = 104.569885
            vertices, faces = silhouette_wood(21, ground, center_x=806)
            wood_report = replace_mesh(wood, vertices, faces, materials=list(wood.data.materials))
            wood_report['source_node'] = wood['source_node']
            for face in wood.data.polygons:
                face.use_smooth = len(face.vertices) == 4
            crown = bpy.data.objects.new('Northern Boundary Tree 21 / Inferred crown', bpy.data.meshes.new('Northern crown'))
            collection.objects.link(crown)
            crown.matrix_world = Matrix.Identity(4)
            for key in ('source_node', 'source_obstacle', 'asset_group', 'asset_name'):
                crown[key] = wood[key]
            crown['projection_component'] = 'crown'
            crown['projection_preserve'] = True
            crown['foliage_physical_opacity'] = True
            crown_report = inferred_crown(crown, center_x=806, ground_y=ground, palette=directory / 'native-northern-leaves.png')
            prepare(worker, asset_id=asset, scene_name='Croisement02 Refinement',
                collection_name='Croisement02 Working', source_path=OUT / 'animation-references/composite-frame-0.png',
                grouping_manifest=reviewed_catalog(), inventory_path=OUT / 'forest-v4-inventory/inventory.json',
                review_path=OUT / 'ownership-revision/grouping-review.json', source_mask_manifest=directory / 'assignments.json',
                width=384, height=384, framing_padding=1.4,
                lighting=dict(toward_sun=[-.45, -.55, .70], ambient=.22, diffuse=.78, shadow_epsilon=.05))
            inspection = worker / 'inspection'
            inspection.mkdir(exist_ok=True)
            evidence = inspection / 'off-map-source'
            evidence.mkdir(exist_ok=True)
            Image.new('RGBA', (1792, 1152)).save(evidence / 'complete-source.png')
            write_json(evidence / 'partition.json', dict(native_bbox=[0, 0, 1792, 1152], observed_foliage_pixels=0,
                evidence='Native wood mask 21 is visible at the northern image edge. Crown and upper continuation are inferred outside the map.'))
            write_json(inspection / 'bark-donor-selection.json', dict(native_mask=21, source_box=[809, 2, 816, 18],
                source_sha256=sha(worker / 'reference/source.png'), reviewer='Codex',
                notes='Inspected enlarged source crop. Upper exposed trunk bark excludes the lower foreground green leaves and ground litter.'))
            write_json(inspection / 'refinement.json', dict(asset_id=asset, mask=21, wood=[wood_report], crown=crown_report,
                source_packet=str(evidence / 'partition.json'), status='geometry candidate; visual review pending',
                limitations=['Obstacle 132 was previously mislabeled as a terrain bank.',
                             'The visible wood follows native mask 21. Upper trunk and full crown outside the map are inferred.',
                             'Inferred leaf appearance uses the native northern canopy palette; no other tree asset is a reference.',
                             'Texture approval and integrated terrain contact remain separate.']))
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            config = json.loads((worker / 'workspace.json').read_text())
            mask_path = Path(config['source_mask_manifest'])
            masks = json.loads(mask_path.read_text())
            assignment = next(a for a in masks['projections']['exterior']['assignments'] if a.get('asset_group') == asset)
            assignment.update(exclude_mask_indices=[134], exclusions_reviewed=True,
                exclusion_reason='Native canopy 134 overlaps 1958 wood-mask pixels, including the green foreground covering the root. Inspected source/alpha overlay; those leaves belong to the canopy, not bark.')
            write_json(mask_path, masks)
            write_json(worker / 'inspection/bark-donor-selection.json', dict(native_mask=21, source_box=[798, 21, 805, 30],
                source_sha256=sha(worker / 'reference/source.png'), reviewer='Codex',
                notes='Inspected upper exposed bark and native foreground overlap. This donor lies inside wood mask 21 and outside foreground canopy 134.'))
            crown = next(o for o in objects if o.get('projection_component') == 'crown')
            crown_report = inferred_crown(crown, center_x=806, ground_y=104.569885,
                                         palette=directory / 'native-northern-leaves.png')
            modified(worker)
            bark = fill(worker, objects, 21, receiver_only=True, donor_mapping='aperiodic-vertical')
            validate(worker)
            bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
            report = json.loads((worker / 'inspection/refinement.json').read_text())
            report.update(model_sha256=sha(worker / 'model.blend'), bark=bark, crown=crown_report)
            write_json(worker / 'inspection/refinement.json', report)
            audit(worker)
            write_json(receipt, dict(model_sha256=report['model_sha256'], native_mask=21, native_obstacle=132,
                previous_id='croisement02-west-root-bank', classification='Tree with explicitly inferred off-map completion'))
        elif json.loads(receipt.read_text())['model_sha256'] != sha(worker / 'model.blend'):
            raise ValueError('Northern tree changed')
        render_workspace(worker, 384, release_slot=False)
    finally:
        release()


if __name__ == '__main__':
    main()
