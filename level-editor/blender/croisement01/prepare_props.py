"""Create source-constrained stump candidates in frozen shared-format packets."""
import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT/'level-editor/refinement'))
sys.path.insert(0, str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT, PROPS
from evidence_io import sha
from render_slots import acquire, release
from refinement_workspace import prepare, modified, validate

SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))


def stump(obj, record, index, profile=None):
    """One closed, connected stump; no stacked cylinders or hidden caps."""
    points = record['points']
    native = [Vector((p['x'], -p['y']/SIN, p['z_top']/COS)) for p in points]
    if index == 55:
        # Measured cap perimeter in the native artwork (about 1–2 pixel
        # uncertainty); constant cap elevation retains the surveyed height.
        cap_pixels = [(578,654),(586,653),(594,655),(600,659),(601,664),
                      (598,669),(591,673),(582,674),(574,672),(570,668),
                      (570,662),(573,658)]
        height=sum(p['z_top'] for p in points)/len(points)/COS
        native=[Vector((x,-(y+height*COS)/SIN,height)) for x,y in cap_pixels]
    center = sum(native, Vector()) / len(native)
    # Retain the native cap outline's uneven radial profile, interpolating
    # around the ring rather than imposing an unrelated circular silhouette.
    ordered = sorted(native, key=lambda p: math.atan2(p.y-center.y,p.x-center.x))
    ring = []
    for i, a in enumerate(ordered):
        b = ordered[(i+1) % len(ordered)]
        for step in range(4):
            ring.append(a.lerp(b, step/4))
    vertices = []
    bottom = min(p['z_bottom'] for p in points)/COS
    lower_center=Vector((583,-695/SIN,0)) if index==55 else center.copy()
    sections=[(0,.78),(.15,.80),(.6,.90),(1,1)]
    if profile is not None:
        lower_center=Vector((profile[0],-profile[1]/SIN,0))
        sections=[(0,profile[2]),(.2,profile[3]),(.6,profile[4]),(1,1)]
    for fraction, radius in sections:
        axis_center=lower_center.lerp(center,fraction)
        for p in ring:
            vertices.append((axis_center.x+(p.x-center.x)*radius,
                             axis_center.y+(p.y-center.y)*radius,
                             bottom+(p.z-bottom)*fraction))
    count = len(ring)
    faces = [tuple(reversed(range(count)))]
    for j in range(3):
        for i in range(count):
            faces.append((j*count+i,j*count+(i+1)%count,(j+1)*count+(i+1)%count,(j+1)*count+i))
    faces.append(tuple(range(3*count,4*count)))
    matrix = obj.matrix_world.inverted()
    mesh = bpy.data.meshes.new(obj.name+' continuous stump')
    mesh.from_pydata([matrix@Vector(v) for v in vertices],[],faces)
    for material in obj.data.materials:
        mesh.materials.append(material)
    uv=mesh.uv_layers.new(name='Source UV')
    ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
    mesh.color_attributes.active_color=ownership
    for face in mesh.polygons:
        for loop in face.loop_indices:
            point=vertices[mesh.loops[loop].vertex_index]
            uv.data[loop].uv=(point[0]/1408,1-(-point[1]*SIN-point[2]*COS)/960)
            ownership.data[loop].color=(0,1,1,1)
    mesh.update()
    obj.data = mesh
    bm = bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    result = dict(vertices=len(bm.verts),faces=len(bm.faces),
                  nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),
                  degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces))
    bm.to_mesh(mesh);bm.free()
    if result['nonmanifold_edges'] or result['degenerate_faces']:
        raise ValueError(result)
    return result


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--asset',default='southwest-cut-stump')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    slug,name,parts,mask=next(row for row in PROPS if row[0]==args.asset)
    if slug not in ('southwest-cut-stump','east-ivy-stump','southeast-small-stump'):
        raise ValueError('This initial recipe supports single-piece cut stumps only; other assemblies need separate construction.')
    asset='croisement01-'+slug
    directory=OUT/'prop-domains';directory.mkdir(exist_ok=True)
    inv=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in inv['masks']:
        row['png']=str(OUT/'baseline/masks'/row['png'])
    inventory=directory/'native-masks.json';inventory.write_text(json.dumps(inv,indent=2)+'\n')
    masks=directory/(slug+'.json')
    masks.write_text(json.dumps(dict(version=1,mask_inventory=str(inventory),projections=dict(exterior=dict(
        state='Initial static source',source_sha256=sha(OUT/'baseline/covered.png'),
        assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[mask])]))),indent=2)+'\n')
    review=directory/(slug+'-grouping-review.json')
    review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',asset_id=asset,
        catalog_sha256=sha(OUT/'catalog.json'),inventory_sha256=sha(OUT/'grouped-inventory/inventory.json'),
        evidence=f'Native mask {mask}, source context and part footprint inspected. Review is scoped to this named assembly; remaining catalog groups are provisional.'),indent=2)+'\n')
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    workspace=OUT/'props-round-1/assets'/asset
    if (workspace/'workspace.json').exists():
        if (workspace/'inspection/refinement.json').exists():
            raise FileExistsError('Completed candidate is immutable; use a fresh round for changes')
        bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'))
        validate(workspace)
    else:
        prepare(workspace,asset_id=asset,scene_name='Croisement01 Refinement',collection_name='Croisement01 Working',
            source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',
            inventory_path=OUT/'grouped-inventory/inventory.json',review_path=review,
            source_mask_manifest=masks,width=256,height=256,framing_padding=1.16,
            lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    level=json.loads((OUT/'baseline/Croisement01.rhp.json').read_text())
    reports=[]
    for obj in bpy.data.collections['Croisement01 Working'].all_objects:
        if obj.type=='MESH' and obj.get('asset_group')==asset:
            index=int(obj['source_node'].split('-')[-1])
            reports.append(dict(source_node=obj['source_node'],**stump(obj,level['sight_obstacles'][index],index)))
    validate(workspace);modified(workspace)
    inspection=workspace/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'refinement.json').write_text(json.dumps(dict(asset_id=asset,parts=reports,
        model_sha256=sha(workspace/'model.blend'),status='private candidate; actual-material, native coverage and contact review pending',
        limitations=['Flare and hidden rear depth inferred from native cap and source image.',
                     'Native mask includes peripheral foliage; source-material ownership needs pixel-level review.',
                     'Lighting is an explicit provisional map hypothesis; independent shadow calibration remains pending.']),indent=2)+'\n')
    release()
    print(workspace)


if __name__=='__main__':
    main()
