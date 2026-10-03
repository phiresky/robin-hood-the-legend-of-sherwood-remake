"""Rebuild three unfinished scenery candidates without changing the other tree packets.

Run with Blender --background --python-exit-code 1 --python this-file -- <new-output>.
The output contains its own scene, proposed domains, catalog and source masks.
The saved scene and all three new eight-view packets still require user review.
"""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/lincoln-refinement'
SELECTED = ('lincoln-bush-west-tower-northwest', 'lincoln-bush-north-curtain-west',
            'lincoln-village-pond-landing-stage')
# Traced on covered.png: the tiled roof below the western bush, and the keep
# wall left of the northern bush. These are ownership corrections, not foliage.
EXCLUSIONS = {
    SELECTED[0]: [[(620, 1105), (620, 1095), (636, 1092), (651, 1086),
                   (665, 1082), (679, 1078), (701, 1078), (723, 1064), (725, 1105)]],
    SELECTED[1]: [[(1780, 350), (1850, 350), (1849, 410), (1847, 470),
                   (1847, 510), (1780, 510)]],
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def revise_domains(output):
    import numpy as np
    from PIL import Image, ImageDraw
    proposal = json.loads((WORK / 'scratch/trees/inventory/tree-catalog-proposal.json').read_text())
    source = Image.open(WORK / 'source-states/covered.png').convert('RGB')
    evidence = []
    for row in proposal['assets']:
        if row['id'] not in EXCLUSIONS:
            continue
        original = Image.open(row['domain_png']).convert('L')
        x, y = row['domain_box_top_left']
        exclusion = Image.new('L', original.size)
        draw = ImageDraw.Draw(exclusion)
        for polygon in EXCLUSIONS[row['id']]:
            draw.polygon([(px - x, py - y) for px, py in polygon], fill=255)
        old = np.array(original) > 0
        revised = old & ~(np.array(exclusion) > 0)
        path = output / 'domains' / f'{row["id"]}.png'
        path.parent.mkdir(parents=True, exist_ok=True)
        Image.fromarray(revised.astype('uint8') * 255).save(path)
        crop = source.crop((x, y, x + original.width, y + original.height))
        overlay = np.array(crop).copy()
        overlay[old & ~revised] = (255, 0, 255)
        sheet = Image.new('RGB', (crop.width * 2, crop.height))
        sheet.paste(crop); sheet.paste(Image.fromarray(overlay), (crop.width, 0))
        sheet.resize((sheet.width * 4, sheet.height * 4), Image.Resampling.NEAREST).save(
            output / 'domains' / f'{row["id"]}-review.png')
        evidence.append(dict(asset_id=row['id'], before_sha256=sha(row['domain_png']),
                             after_sha256=sha(path), removed_pixels=int((old & ~revised).sum()),
                             source_sha256=sha(WORK / 'source-states/covered.png'),
                             exclusion_polygons=EXCLUSIONS[row['id']]))
        row.update(domain_png=str(path), domain_pixels=int(revised.sum()),
                   domain_method=row['domain_method'] + '; roof/keep-wall pixels excluded by source trace (scenery revision 3)')
    (output / 'domain-revision.json').write_text(json.dumps(evidence, indent=2) + '\n')
    path = output / 'tree-catalog-proposal.json'
    path.write_text(json.dumps(proposal, indent=2) + '\n')
    # The catalog consumes the set of terrain masks and recomputes every overlap
    # against the revised domains; none of the previous pixel counts are reused.
    shutil.copy2(WORK / 'scratch/trees/inventory/ground-domain-trim.json', output / 'ground-domain-trim.json')
    return path, proposal


def fit_landing_posts(collection, report):
    """Fit the two visible post widths to the independently traced source domain."""
    matches = [o for o in collection.all_objects if o.get('asset_group') == SELECTED[2]
               and o.get('projection_component') == 'posts']
    if len(matches) != 1 or len(matches[0].data.vertices) != 80:
        raise ValueError('Expected four deck posts and one eight-sided mooring pole')
    obj = matches[0]
    # The painted front-left post spans x=505..511, whereas the old vertical
    # post was centred at the deck corner x=506. The pole spans x=540..545.
    for start, stop, centre, shift, scale in [(48, 64, 506., 2., 3. / 2.2),
                                               (64, 80, 542., .5, 2.8 / 2.)]:
        for vertex in list(obj.data.vertices)[start:stop]:
            vertex.co.x = centre + shift + (vertex.co.x - centre) * scale
    obj.data.update()
    report['post_source_fit'] = dict(front_left_x=[505, 511], mooring_pole_x=[540, 545],
                                     method='Source-traced post widths and centres; unchanged deck and ground contacts')
    decks = [o for o in collection.all_objects if o.get('asset_group') == SELECTED[2]
             and o.get('projection_component') == 'deck']
    if len(decks) != 1 or len(decks[0].data.polygons) != 6:
        raise ValueError('Expected one six-face landing deck')
    deck = decks[0].data
    if deck.polygons[0].normal.z >= 0:
        raise ValueError('Landing deck winding no longer matches the repair input')
    for face in deck.polygons:
        face.flip()
    deck.update()
    if deck.polygons[0].normal.z <= 0:
        raise ValueError('Landing deck top must face upward')
    report['deck_winding_repaired'] = 'Outward face normals; unchanged vertices and silhouette'


def main(output):
    output = Path(output).resolve()
    if output.exists():
        raise FileExistsError(output)
    output.mkdir(parents=True)
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    sys.path.insert(0, str(ROOT / 'level-editor/blender/lincoln'))
    from render_slots import acquire
    acquire()
    import bpy
    from mathutils.bvhtree import BVHTree
    import trees_geometry as geometry
    from freeze_tooling import select_tooling
    select_tooling(WORK / 'tooling-trees/e48c662ac3828db9')
    from refinement_inventory import inventory
    from foliage_trees import geometry_hash

    proposal_path, proposal = revise_domains(output)
    scene_dir = output / 'scene'
    scene_dir.mkdir()
    previous = json.loads((WORK / 'trees/scene-v2/scene-report.json').read_text())
    if sha(previous['output_blend']) != previous['output_blend_sha256']:
        raise ValueError('Previous trees scene differs from its saved report')
    bpy.ops.wm.open_mainfile(filepath=previous['output_blend'], load_ui=False)
    bpy.context.window.scene = bpy.data.scenes['lincoln Refinement']
    collection = bpy.data.collections['lincoln Working']
    unchanged = {o.name: geometry_hash(o) for o in collection.all_objects
                 if o.type == 'MESH' and o.get('asset_group') not in SELECTED}
    for obj in list(collection.all_objects):
        if obj.get('asset_group') in SELECTED:
            bpy.data.objects.remove(obj, do_unlink=True)
    static = [o for o in collection.all_objects if o.type == 'MESH' and not o.hide_render
              and not str(o.get('source_node', '')).startswith(('foliage-', 'scenery-'))]
    terrain_nodes = geometry.terrain_nodes()
    terrain = [o for o in static if o.get('source_node') in terrain_nodes
               or any(word in str(o.get('asset_group', '')) for word in geometry.NATURAL_GROUP_WORDS)]

    def bvh(objects):
        verts, tris = [], []
        for obj in objects:
            obj.data.calc_loop_triangles()
            offset = len(verts)
            verts.extend(obj.matrix_world @ v.co for v in obj.data.vertices)
            tris.extend(tuple(offset + i for i in t.vertices) for t in obj.data.loop_triangles)
        return BVHTree.FromPolygons(verts, tris, all_triangles=True)

    terrain_tree, architecture_tree, static_tree = bvh(terrain), bvh([o for o in static if o not in terrain]), bvh(static)
    image = bpy.data.images.load(str(geometry.SOURCE), check_existing=True)
    rows = {r['id']: r for r in proposal['assets']}
    reports = {r['asset_id']: r for r in previous['assets']}
    for number, identifier in enumerate(SELECTED):
        row = rows[identifier]
        if row['kind'] == 'scenery':
            reports[identifier] = geometry.build_landing_stage(row, terrain_tree, static_tree, collection, image)
            fit_landing_posts(collection, reports[identifier])
        else:
            reports[identifier] = geometry.build_foliage(row, terrain_tree, architecture_tree, static_tree,
                                                        collection, image, scene_dir / 'foliage-source' / identifier,
                                                        11000 + number)
        print('SCENERY_REBUILT', identifier, json.dumps(reports[identifier]['depth_fit']), flush=True)
    after = {o.name: geometry_hash(o) for o in collection.all_objects
             if o.type == 'MESH' and o.get('asset_group') not in SELECTED}
    if unchanged != after:
        raise ValueError('Unrelated geometry changed during scenery repair')
    bpy.context.view_layer.update()
    blend = scene_dir / 'lincoln-grouped-trees.blend'
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(blend), compress=True)
    scene_inventory = inventory(scene_dir / 'inventory', collection_name='lincoln Working', map_name='lincoln',
                                source_path=str(geometry.SOURCE))
    record = dict(previous, recipe=str(Path(__file__).resolve()), recipe_sha256=sha(__file__),
                  output_blend=str(blend), output_blend_sha256=sha(blend), inventory=scene_inventory,
                  assets=list(reports.values()), repaired_assets=list(SELECTED), unrelated_geometry_preserved=True,
                  proposal=str(proposal_path), proposal_sha256=sha(proposal_path))
    (scene_dir / 'scene-report.json').write_text(json.dumps(record, indent=2) + '\n')
    import trees_catalog
    trees_catalog.OUT = output
    trees_catalog.PROPOSAL = proposal_path
    trees_catalog.SCENE_INVENTORY = scene_dir / 'inventory/inventory.json'
    sys.argv = [__file__]
    trees_catalog.main()
    print('SCENERY_REPAIR_COMPLETE', str(output), flush=True)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1])
