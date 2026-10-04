"""Isolated authored wood candidates for native masks without sight obstacles.

These are supplemental stem hypotheses, not approved complete trees. They do
not mutate the map catalog, existing tree workers, or terrain ownership.
"""
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_inventory import inventory, validate_catalog
from refinement_workspace import prepare, validate, modified
from complete_north_tree23 import silhouette_wood
from tree_geometry import replace_mesh, SIN, COS
from audit_candidates import audit
from render_tree import render_workspace


def proposal():
    directory = OUT / 'missing-wood-review'
    directory.mkdir(exist_ok=True)
    level = json.loads((OUT / 'baseline/Croisement02.rhp.json').read_text())
    source = Image.open(OUT / 'animation-references/composite-frame-0.png').convert('RGB')
    sheet = Image.new('RGB', (1200, 620), '#333333')
    draw = ImageDraw.Draw(sheet)
    for column, index in enumerate((9, 22, 44)):
        record = level['masks'][index]
        x, y = record['box_top_left']; width, height = record['box_size']
        context = source.crop((max(0, x-100), max(0, y-60), min(1792, x+width+100), min(1152, y+height+60)))
        context.thumbnail((390, 390)); sheet.paste(context, (column*400, 20))
        cutout = source.crop((x, y, x+width, y+height)).convert('RGBA')
        cutout.putalpha(Image.open(OUT / f'baseline/masks/{index:06}.png').convert('L'))
        cutout.thumbnail((390, 200)); sheet.paste(cutout, (column*400, 410), cutout)
        draw.text((column*400, 0), f'Wood mask {index}: native source context and cutout', fill='white')
    sheet.save(directory / 'source-context.png')
    def bitmap(index):
        r = level['masks'][index]
        x, y = r['box_top_left']; w, h = r['box_size']
        result = np.zeros((1152, 1792), bool)
        result[y:y+h, x:x+w] = np.asarray(Image.open(OUT / f'baseline/masks/{index:06}.png').convert('L')) > 0
        return result
    rows = []
    decisions = {
        9: 'Separate thin stem beside tree08; no shared root is visible. Supplemental authored wood; crown ownership remains unresolved.',
        22: 'Northern clipped mask is visually foliage, including portions outside native canopy masks. Do not automatically project those pixels onto bark or invent a standalone tree.',
        44: 'Separate forked stem beside tree45. Its base extends beyond the south map boundary. Supplemental authored wood; crown ownership remains unresolved.',
    }
    for index in (9, 22, 44):
        own = bitmap(index)
        overlaps = {str(i): int(np.count_nonzero(own & bitmap(i))) for i in list(range(54, 94)) + list(range(128, 136)) if np.any(own & bitmap(i))}
        rows.append(dict(native_mask=index, native_mask_sha256=sha(OUT / f'baseline/masks/{index:06}.png'),
                         pixels=int(own.sum()), foreground_overlap_pixels=overlaps, decision=decisions[index],
                         native_sight_obstacles=level['masks'][index]['obstacle_indices']))
    result = dict(status='source ownership reviewed; supplemental geometry candidates pending', reviewer='Codex',
                  source_sha256=sha(OUT / 'animation-references/composite-frame-0.png'), masks=rows,
                  source_context_sha256=sha(directory / 'source-context.png'),
                  integration_requirements=['Resolve crown associations against approved trees08 and45 without duplicate foliage.',
                                           'Introduce authored source nodes through canonical catalog and fresh inventory.',
                                           'Add authored domains, terrain exclusions, and scoped occluder constraints.',
                                           'Do not generate textures or publish before new geometry approval.'])
    write_json(directory / 'ownership-proposal.json', result)
    return directory


def build(index):
    directory = proposal() / f'wood-{index:02}'
    directory.mkdir(exist_ok=True)
    asset = f'croisement02-supplemental-wood-{index:02}'
    worker = OUT / 'missing-wood-round-1/assets' / asset
    if worker.exists():
        raise FileExistsError(worker)
    bpy.ops.wm.open_mainfile(filepath=str(OUT / 'forest-v4-input.blend'))
    bpy.context.preferences.filepaths.save_version = 0
    collection = bpy.data.collections['Croisement02 Working']
    # Keep the source inventory's existing ownership; this private catalog only
    # adds the new source node. Corrected map ownership is integrated separately.
    catalog = json.loads((OUT / 'catalog.json').read_text())
    node = f'foliage-wood-{index:03}'
    name = f'Supplemental native wood {index:02}'
    catalog['groups'].append(dict(id=asset, name=name, parts=[dict(node=node, name=name)]))
    write_json(directory / 'catalog.json', catalog)
    obj = bpy.data.objects.new(name, bpy.data.meshes.new(name))
    collection.objects.link(obj)
    for key, value in dict(source_node=node, asset_group=asset, asset_name=name, part_name=name).items():
        obj[key] = value
    ground = 414 if index == 9 else 1180
    vertices, faces = silhouette_wood(index, ground)
    if index == 44:
        # Extend each cropped bottom ring beyond the image rather than leaving
        # a cap on the image boundary. The native in-map rings remain unchanged.
        for face_index in range(len(faces) - 1, -1, -1):
            face = faces[face_index]
            projected = [-vertices[i][1] * SIN - vertices[i][2] * COS for i in face]
            if len(face) != 24 or min(projected) < 1151.99:
                continue
            faces.pop(face_index)
            start = len(vertices)
            center_x = sum(vertices[i][0] for i in face) / 24
            for i in face:
                p = vertices[i].copy()
                p[0] = center_x + (p[0] - center_x) * 1.15
                p[2] -= 30 / COS
                vertices.append(p)
            for j in range(24):
                faces.append((face[j], face[(j+1) % 24], start+(j+1) % 24, start+j))
            faces.append(tuple(start+j for j in range(24)))
    template = next(o for o in collection.all_objects if o.type == 'MESH' and o.get('source_node') == 'building-059')
    geometry = replace_mesh(obj, vertices, faces, materials=list(template.data.materials))
    for face in obj.data.polygons:
        face.use_smooth = len(face.vertices) == 4
    inventory(directory / 'inventory', collection_name='Croisement02 Working', map_name='Croisement02',
              source_path=OUT / 'animation-references/composite-frame-0.png')
    validate_catalog(directory / 'inventory/inventory.json', directory / 'catalog.json')
    write_json(directory / 'grouping-review.json', dict(status='reviewed', reviewer='Codex',
        catalog_sha256=sha(directory / 'catalog.json'), inventory_sha256=sha(directory / 'inventory/inventory.json'),
        evidence='Source artwork, native individual wood masks and layer0 occlusion thresholds reviewed. New node owns only its distinct stem; crown grouping remains pending and no approved tree is modified.'))
    masks = json.loads((OUT / 'forest-v4-round-1/assets/croisement02-tree-08/source-masks.json').read_text())
    exclusions = [133] if index == 9 else [89, 128]
    masks['projections']['exterior']['assignments'].append(dict(source_node=node, mask_indices=[index], reviewed=True,
        exclude_mask_indices=exclusions, exclusions_reviewed=True,
        exclusion_reason='Visually inspected native foreground leaves crossing the wood silhouette; leaves are not bark.'))
    write_json(directory / 'source-masks.json', masks)
    prepare(worker, asset_id=asset, scene_name='Croisement02 Refinement', collection_name='Croisement02 Working',
        source_path=OUT / 'animation-references/composite-frame-0.png', grouping_manifest=directory / 'catalog.json',
        inventory_path=directory / 'inventory/inventory.json', review_path=directory / 'grouping-review.json',
        source_mask_manifest=directory / 'source-masks.json', width=256, height=384, framing_padding=1.25)
    inspection = worker / 'inspection'
    inspection.mkdir(exist_ok=True)
    packet = inspection / 'source-domain'
    packet.mkdir(exist_ok=True)
    Image.new('RGBA', (1792, 1152)).save(packet / 'complete-source.png')
    write_json(packet / 'partition.json', dict(native_bbox=[0, 0, 1792, 1152], observed_foliage_pixels=0))
    write_json(inspection / 'refinement.json', dict(asset_id=asset, mask=index, wood=[geometry],
        model_sha256=sha(worker / 'model.blend'), source_packet=str(packet / 'partition.json'),
        status='supplemental wood candidate; incomplete logical tree; not ready for approval',
        limitations=['No crown assigned: association with neighbouring approved tree must be resolved.',
                     'Unknown bark remains neutral in source-only materials.',
                     'Native mask shape is inferred as a closed rounded cross section; depth is not recovered.',
                     'Authored scenery has no native sight obstacle; terrain integration remains pending.']))
    finalize(worker)


def finalize(worker):
    inspection = worker / 'inspection'
    for obj in bpy.data.collections['Croisement02 Working'].all_objects:
        if obj.type != 'MESH' or obj.get('asset_group') != worker.name or obj.get('wood_smoothing'):
            continue
        bpy.ops.object.select_all(action='DESELECT')
        obj.select_set(True); bpy.context.view_layer.objects.active = obj
        smooth = obj.modifiers.new('Soften native raster steps', 'SMOOTH')
        smooth.factor = .45; smooth.iterations = 8
        bpy.ops.object.modifier_apply(modifier=smooth.name)
        obj['wood_smoothing'] = 'Eight gentle surface smoothing passes; source coverage independently measured'
    modified(worker)
    bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
    report = json.loads((inspection / 'refinement.json').read_text())
    report['model_sha256'] = sha(worker / 'model.blend')
    write_json(inspection / 'refinement.json', report)
    validate(worker)
    audit(worker)
    render_workspace(worker, 256, release_slot=False)
    print('SUPPLEMENTAL WOOD', worker, flush=True)


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    acquire()
    try:
        if args and args[0] == 'finalize':
            worker = Path(args[1]).resolve()
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            finalize(worker)
            args = []
        else:
            args = args or ['9', '44']
        for index in map(int, args):
            if index not in (9, 44):
                raise ValueError('Only visually supported wood09 and44 are modeled')
            build(index)
    finally:
        release()
