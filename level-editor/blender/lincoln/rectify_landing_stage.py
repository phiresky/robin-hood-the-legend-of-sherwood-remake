"""Fit a truly rectangular landing deck to the four source-art corner traces."""
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/lincoln-refinement'
ASSET = 'lincoln-village-pond-landing-stage'
# Least-squares fit in source pixels, constrained to perpendicular world axes.
# The trace distinguishes the top deck from its shadow and thickness below it.
CENTER = (516., 640.25)
ANGLE = -.3526447077511208
HALF_WIDTH = 21.64052294769571
HALF_DEPTH = 26.58796978162973


def main(output):
    import bpy
    from mathutils import Vector
    sys.path.insert(0, str(ROOT / 'level-editor/blender/lincoln'))
    from render_slots import acquire
    from freeze_tooling import select_tooling
    acquire()
    select_tooling(WORK / 'tooling-trees/e48c662ac3828db9')
    from refinement_inventory import inventory
    from refinement_workspace import _geometry
    import trees_catalog
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    previous = WORK / 'trees/revision-5'
    record = json.loads((previous / 'scene/scene-report.json').read_text())
    sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
    if sha(record['output_blend']) != record['output_blend_sha256']:
        raise ValueError('Reviewed source scene changed')
    bpy.ops.wm.open_mainfile(filepath=record['output_blend'], load_ui=False)
    collection = bpy.data.collections['lincoln Working']
    protected = {o.name: _geometry(o) for o in collection.all_objects if o.get('asset_group') != ASSET}
    parts = {o.get('projection_component'): o for o in collection.all_objects if o.get('asset_group') == ASSET}
    deck, posts = parts['deck'], parts['posts']
    if len(deck.data.vertices) != 8 or len(posts.data.vertices) != 80:
        raise ValueError('Unexpected landing topology')
    sine = math.sin(math.radians(35))
    u = Vector((math.cos(ANGLE), math.sin(ANGLE)*sine)) * HALF_WIDTH
    v = Vector((-math.sin(ANGLE), math.cos(ANGLE)*sine)) * HALF_DEPTH
    corners = [Vector(CENTER) + i*u + j*v for i,j in [(-1,-1),(1,-1),(1,1),(-1,1)]]
    traces = [(486,630),(529,621),(543,648),(506,662)]
    deltas=[]
    for index, (corner, traced) in enumerate(zip(corners,traces)):
        delta=Vector((corner.x-traced[0],-(corner.y-traced[1])/sine,0))
        deltas.append(list(delta))
        for k in (index,index+4):deck.data.vertices[k].co += delta
        # Keep every support under its corresponding deck corner; move both
        # ends together so the post stays vertical and retains its ground z.
        for vertex in list(posts.data.vertices)[index*16:(index+1)*16]:vertex.co += delta
    deck.data.update();posts.data.update()
    a=deck.data.vertices[1].co-deck.data.vertices[0].co
    b=deck.data.vertices[3].co-deck.data.vertices[0].co
    c=deck.data.vertices[2].co-deck.data.vertices[1].co
    if abs(a.normalized().dot(b.normalized()))>1e-4 or (c-b).length>1e-3:
        raise ValueError(f'Landing deck is not a rectangle: dot={a.normalized().dot(b.normalized())}, opposite-edge error={(c-b).length}')
    if protected != {o.name:_geometry(o) for o in collection.all_objects if o.get('asset_group') != ASSET}:
        raise ValueError('Unrelated geometry changed')
    scene_dir=output/'scene';scene_dir.mkdir()
    blend=scene_dir/'lincoln-grouped-trees.blend'
    bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(blend),compress=True)
    inv=inventory(scene_dir/'inventory',collection_name='lincoln Working',map_name='lincoln',source_path=str(WORK/'trees/revision-4/source-covered.png'))
    for row in record['assets']:
        if row['asset_id']==ASSET:
            row['rectangular_deck']=dict(source_corners=[list(c) for c in corners],source_corner_traces=traces,
                world_edge_dot=float(a.normalized().dot(b.normalized())),corner_displacements=deltas,
                world_width=float(a.length),world_depth=float(b.length),method='Constrained orthographic rectangle fit')
    record.update(output_blend=str(blend),output_blend_sha256=sha(blend),recipe=str(Path(__file__).resolve()),recipe_sha256=sha(__file__),inventory=inv,repaired_assets=[ASSET],unrelated_geometry_preserved=True)
    (scene_dir/'scene-report.json').write_text(json.dumps(record,indent=2)+'\n')
    for name in ['tree-catalog-proposal.json','ground-domain-trim.json']:shutil.copy2(previous/name,output/name)
    trees_catalog.OUT=output;trees_catalog.PROPOSAL=output/'tree-catalog-proposal.json';trees_catalog.SCENE_INVENTORY=scene_dir/'inventory/inventory.json'
    sys.argv=[__file__];trees_catalog.main()
    print('RECTANGULAR_DECK',json.dumps(next(r['rectangular_deck'] for r in record['assets'] if r['asset_id']==ASSET)),flush=True)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--')+1])
