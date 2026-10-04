"""Infer an off-map crown above the observed northern trunk clump."""
import json
import math
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
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import prepare, validate, modified
from tree_geometry import replace_mesh, SIN, COS, RAY
from bark_materials import fill
from audit_candidates import audit
from render_tree import render_workspace


def silhouette_wood(mask, ground_y, center_x=616):
    """Sweep each observed row span; continue cropped upper ends beyond the map."""
    level = json.loads((OUT / 'baseline/Croisement02.rhp.json').read_text())
    record = level['masks'][mask]
    x0, y0 = record['box_top_left']
    alpha = np.asarray(Image.open(OUT / f'baseline/masks/{mask:06}.png').convert('L')) > 0
    tracks, active = [], []
    for y, row in enumerate(alpha):
        changes = np.diff(np.pad(row.astype(int), 1))
        spans = list(zip(np.flatnonzero(changes == 1), np.flatnonzero(changes == -1)))
        available = set(active)
        following = []
        for left, right in spans:
            center, radius = x0 + (left + right) / 2, (right - left) / 2
            candidates = [(abs(tracks[i][-1][0] - center), i) for i in available
                          if abs(tracks[i][-1][0] - center) <= tracks[i][-1][2] + radius + 1]
            if candidates:
                _, index = min(candidates)
                available.remove(index)
            else:
                index = len(tracks)
                tracks.append([])
            tracks[index].append((center, y0 + y + .5, radius))
            following.append(index)
        active = following
    vertices, faces = [], []
    for track in tracks:
        x, y, radius = track[0]
        path = [(x, y - .5, radius), *track, (track[-1][0], track[-1][1] + .5, track[-1][2])]
        if y <= .5:
            path = [(x + (x - center_x) * .3, -100, 2.), (x, -20, radius), *path]
        start = len(vertices)
        sides = 24
        for x, y, radius in path:
            center = np.array([x, -ground_y / SIN, (ground_y - y) / COS])
            for side in range(sides):
                angle = math.tau * side / sides
                point = center + np.array([radius * math.cos(angle), 0, 0]) + np.asarray(RAY) * (radius / COS * math.sin(angle))
                vertices.append(point.tolist())
        faces.append(tuple(start + i for i in reversed(range(sides))))
        for row in range(len(path) - 1):
            for side in range(sides):
                a = start + row * sides + side
                b = start + row * sides + (side + 1) % sides
                faces.append((a, b, b + sides, a + sides))
        faces.append(tuple(start + (len(path) - 1) * sides + i for i in range(sides)))
    return vertices, faces


def inferred_crown(crown, *, center_x=616, ground_y=73, palette=None):
    old = OUT / 'forest-v4-round-1/assets/croisement02-tree-23'
    report = json.loads((old / 'inspection/refinement.json').read_text())
    packet = Path(report['source_packet'])
    palette = Path(palette) if palette else packet.parent / 'complete-source.png'
    image = Image.open(palette).convert('RGBA')
    alpha = np.asarray(image)[:, :, 3] > 127
    height, width = alpha.shape
    patches = [(x, y) for y in range(0, height - 24, 8) for x in range(0, width - 24, 8)
               if alpha[y:y + 24, x:x + 24].mean() > .55]
    if not patches:
        raise ValueError('No sufficiently leafy native samples')
    from tree_geometry import material
    mat = material('Inferred northern crown leaf samples', palette, False)
    rng = np.random.default_rng(23021)
    vertices, faces, uvs = [], [], []
    for _ in range(700):
        unit = rng.normal(size=3)
        unit /= np.linalg.norm(unit)
        unit *= rng.uniform(.05, 1.) ** (1 / 3)
        center = np.array([center_x, -ground_y / SIN, (ground_y + 172) / COS]) + unit * [84., 98., 76.]
        px, py = patches[int(rng.integers(len(patches)))]
        uv = [(px / width, 1 - py / height), ((px + 24) / width, 1 - py / height),
              ((px + 24) / width, 1 - (py + 24) / height), (px / width, 1 - (py + 24) / height)]
        size = rng.uniform(11., 17.)
        for a, b in [(0, 1), (0, 2), (1, 2)]:
            start = len(vertices)
            for sa, sb in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
                p = center.copy()
                p[a] += sa * size
                p[b] += sb * size
                vertices.append(p.tolist())
            faces.extend([(start, start + 1, start + 2), (start, start + 2, start + 3)])
            uvs.extend(uv)
    points = np.asarray(vertices)
    if np.max(-points[:, 1] * SIN - points[:, 2] * COS) >= 0:
        raise ValueError('Inferred crown must remain outside observed map pixels')
    result = replace_mesh(crown, vertices, faces, uvs, [mat], [0] * len(faces), [False] * len(faces))
    result.update(geometry_version='native-leaf-clusters-v5', width=float(np.ptp(points[:, 0])),
                  depth=float(np.ptp(points[:, 1])), source_projection_preserved=False,
                  inferred_off_map_crown=True, leaf_clusters=700,
                  tree_references=['leicester-southeast-cottage-tree', 'leicester-moat-bank-tree'])
    return result


def main():
    asset = 'croisement02-tree-23'
    old = OUT / 'forest-v4-round-1/assets' / asset
    worker = OUT / 'forest-v4-round-2/assets' / asset
    receipt = worker / 'inspection/source-domain-revision.json'
    latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    if latest.get(asset, {}).get('decision') == 'approved':
        raise ValueError('Approved geometry is frozen')
    acquire()
    try:
        if receipt.exists() and '--redo' in sys.argv:
            receipt.rename(receipt.with_name('source-domain-revision-archive-' + uuid.uuid4().hex[:8] + '.json'))
        if not (worker / 'workspace.json').exists():
            bpy.ops.wm.open_mainfile(filepath=str(old / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            crown = next(o for o in objects if o.get('projection_component') == 'crown')
            wood = next(o for o in objects if o.get('projection_component') != 'crown')
            result = inferred_crown(crown)
            vertices, faces = silhouette_wood(23, 72.840874)
            wood_report = replace_mesh(wood, vertices, faces, materials=list(wood.data.materials))
            wood_report['source_node'] = wood['source_node']
            prepare(worker, asset_id=asset, scene_name='Croisement02 Refinement',
                collection_name='Croisement02 Working',
                source_path=OUT / 'animation-references/composite-frame-0.png',
                grouping_manifest=OUT / 'catalog.json', inventory_path=OUT / 'forest-v4-inventory/inventory.json',
                review_path=OUT / 'forest-v4-grouping-review.json', source_mask_manifest=old / 'source-masks.json',
                width=384, height=384, framing_padding=1.4,
                lighting=dict(toward_sun=[-.45, -.55, .70], ambient=.22, diffuse=.78, shadow_epsilon=.05))
            directory = worker / 'inspection/off-map-source'
            directory.mkdir(parents=True)
            Image.new('RGBA', (1792, 1152)).save(directory / 'complete-source.png')
            write_json(directory / 'partition.json', dict(native_bbox=[0, 0, 1792, 1152],
                observed_foliage_pixels=0, evidence='The three visible trunks terminate at the top image edge. No in-map foliage is assigned to this clump; its upper branches and crown are inferred.'))
            report = json.loads((old / 'inspection/refinement.json').read_text())
            report.update(crown=result, wood=[wood_report], source_packet=str(directory / 'partition.json'))
            report['limitations'].append('Entire crown and upper branches lie beyond the north image edge and are inferred. Shared Croisement02 canopy samples provide inferred leaf appearance only; the former disconnected in-map crown is removed.')
            write_json(worker / 'inspection/refinement.json', report)
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            crown = next(o for o in objects if o.get('projection_component') == 'crown')
            crown_report = inferred_crown(crown)
            wood = next(o for o in objects if o.get('projection_component') != 'crown')
            vertices, faces = silhouette_wood(23, 72.840874)
            wood_report = replace_mesh(wood, vertices, faces, materials=list(wood.data.materials))
            wood_report['source_node'] = wood['source_node']
            for face in wood.data.polygons:
                face.use_smooth = len(face.vertices) == 4
            write_json(worker / 'inspection/bark-donor-selection.json', dict(native_mask=23,
                source_box=[601, 10, 607, 29], source_sha256=sha(worker / 'reference/source.png'),
                reviewer='Codex', notes='Inspected enlarged source crop: pale birch bark and its dark transverse markings, excluding the ground foliage below the trunks.'))
            modified(worker)
            bark = fill(worker, objects, 23, receiver_only=True)
            validate(worker)
            bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
            report = json.loads((worker / 'inspection/refinement.json').read_text())
            report.update(model_sha256=sha(worker / 'model.blend'), bark=bark, crown=crown_report, wood=[wood_report])
            write_json(worker / 'inspection/refinement.json', report)
            audit(worker)
            write_json(receipt, dict(model_sha256=report['model_sha256'],
                previous_model_sha256=sha(old / 'model.blend'), correction='Disconnected in-map crown replaced with explicitly inferred off-map completion'))
        elif json.loads(receipt.read_text())['model_sha256'] != sha(worker / 'model.blend'):
            raise ValueError('Northern tree revision changed')
        render_workspace(worker, 384, release_slot=False)
    finally:
        release()


if __name__ == '__main__':
    main()
