"""Add inferred image-edge crown continuations in new, unapproved workspaces."""
import argparse
import hashlib
import json
import math
import sys
import uuid
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Vector
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, tree_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, COS, material
from refinement_workspace import prepare, modified, validate
from audit_candidates import audit
from render_tree import render_workspace


def mesh_prefix(mesh, vertices=None, faces=None, loops=None):
    vertices = len(mesh.vertices) if vertices is None else vertices
    faces = len(mesh.polygons) if faces is None else faces
    loops = len(mesh.loops) if loops is None else loops
    payload = dict(vertices=[list(v.co) for v in list(mesh.vertices)[:vertices]],
        faces=[dict(vertices=list(p.vertices),slot=p.material_index) for p in list(mesh.polygons)[:faces]],
        uv={layer.name:[list(v.uv) for v in list(layer.data)[:loops]] for layer in mesh.uv_layers},
        ownership=[list(v.color) for v in list(mesh.color_attributes['Source ownership'].data)[:loops]])
    return hashlib.sha256(json.dumps(payload,sort_keys=True).encode()).hexdigest()


def cap(crown, packet_path, mask, destination, edge='north'):
    counts = (len(crown.data.vertices),len(crown.data.polygons),len(crown.data.loops))
    observed_before = mesh_prefix(crown.data)
    packet = json.loads(packet_path.read_text())
    source_path = packet_path.parent / 'complete-source.png'
    rgba = np.asarray(Image.open(source_path).convert('RGBA'))
    x, y, width, height = packet['native_bbox']
    alpha = rgba[:, :, 3] > 127
    if edge=='north':
        if not 0 <= -y < height:raise ValueError('Source domain does not reach the north edge')
        edge_x = np.where(alpha[-y])[0] + x
    elif edge=='east':
        if not 0 <= 1791-x < width:raise ValueError('Source domain does not reach the east edge')
        edge_x = np.where(alpha[:,1791-x])[0] + y
    else:
        raise ValueError('Unsupported map edge')
    if len(edge_x) < 8:
        raise ValueError('Insufficient northern crown contact for this recipe')
    left, right = float(edge_x.min()), float(edge_x.max())
    center_x, radius_x = (left + right) / 2, (right - left) / 2 + 10
    world = np.array([tuple(crown.matrix_world @ v.co) for v in crown.data.vertices])
    center_y = (world[:, 1].min() + world[:, 1].max()) / 2
    radius_y = np.ptp(world[:, 1]) / 2
    rise = max(45., min(145., radius_x * .95))
    if edge=='east':radius_y=max(radius_y,(np.ptp(world[:,0])+rise)*.60)
    else:radius_y=min(radius_y,radius_x*1.15)
    patch_ys=range(max(0,-y),min(height-24,-y+130),8) if edge=='north' else range(0,height-24,8)
    patch_xs=range(0,width-24,8) if edge=='north' else range(max(0,width-130),width-24,8)
    patches = [(px, py) for py in patch_ys for px in patch_xs if alpha[py:py+24, px:px+24].mean() > .55]
    if not patches:
        raise ValueError('No local native leaf palette')
    rng = np.random.default_rng(74000 + mask)
    atlas = np.zeros((48*24, 64*24, 4), np.uint8)
    tiles, new_vertices, new_faces, new_uv = 0, [], [], []
    sample_v, sample_u = np.mgrid[0:24, 0:24] / 24 + .5 / 24
    for _ in range(900):
        direction = rng.normal(size=3)
        direction /= np.linalg.norm(direction)
        direction *= rng.uniform(.02, 1.) ** (1/3)
        angle = math.atan2(direction[1], direction[0])
        irregular = 1 + .08 * math.sin(5*angle + mask) + .045 * math.cos(9*angle)
        if edge=='north':
            center=np.array([center_x,center_y,(-center_y*SIN+rise*.15)/COS])
            position=center+direction*np.array([radius_x,radius_y,rise])*irregular
            if -position[1]*SIN-position[2]*COS>=0:continue
        else:
            center=np.array([1792+rise*.15,center_y,(-center_x-center_y*SIN)/COS])
            position=center+direction*np.array([rise,radius_y,radius_x])*irregular
            if position[0]<=1792:continue
        axis = Vector(rng.normal(size=3)).normalized()
        other = axis.cross(Vector((0, 0, 1)) if abs(axis.z) < .9 else Vector((1, 0, 0))).normalized()
        third = axis.cross(other).normalized()
        size = rng.uniform(7, 12)
        px, py = patches[int(rng.integers(len(patches)))]
        for u, v in [(axis, other), (axis, third), (other, third)]:
            points = [position + size*(np.asarray(u)*su + np.asarray(v)*sv)
                      for su, sv in [(-1,-1),(1,-1),(1,1),(-1,1)]]
            for p in points:
                if edge=='north':
                    projected_y = -p[1]*SIN - p[2]*COS
                    if projected_y >= -.01:p[2] += (projected_y + .01) / COS
                else:p[0]=max(p[0],1792.01)
            p0, p1, p2, p3 = points
            samples = ((1-sample_u)[...,None]*(1-sample_v)[...,None]*p0
                + sample_u[...,None]*(1-sample_v)[...,None]*p1
                + sample_u[...,None]*sample_v[...,None]*p2
                + (1-sample_u)[...,None]*sample_v[...,None]*p3)
            projected = -samples[...,1]*SIN - samples[...,2]*COS
            ix = np.floor(samples[...,0]-x).astype(int)
            iy = np.floor(-projected-y).astype(int)
            if edge=='east':
                ix=np.floor(3584-samples[...,0]-x).astype(int)
                iy=np.floor(projected-y).astype(int)
            valid = (ix>=0)&(ix<width)&(iy>=0)&(iy<height)
            weight = np.clip(1 + projected/24, 0, 1)
            if edge=='east':weight=np.clip(1-(samples[...,0]-1792)/24,0,1)
            tile = rgba[py:py+24,px:px+24].copy()
            edge_alpha = np.zeros((24,24))
            edge_alpha[valid] = alpha[iy[valid],ix[valid]]
            tile[...,3] = np.rint(tile[...,3]*(1-weight+weight*edge_alpha)).astype(np.uint8)
            blend = weight[valid,None]
            tile[...,:3][valid] = np.rint(tile[...,:3][valid]*(1-blend)
                + rgba[iy[valid],ix[valid],:3]*blend).astype(np.uint8)
            ax, ay = tiles%64*24, tiles//64*24
            atlas[ay:ay+24,ax:ax+24] = tile
            coords = [(ax/atlas.shape[1],1-ay/atlas.shape[0]),((ax+24)/atlas.shape[1],1-ay/atlas.shape[0]),
                ((ax+24)/atlas.shape[1],1-(ay+24)/atlas.shape[0]),(ax/atlas.shape[1],1-(ay+24)/atlas.shape[0])]
            start = len(new_vertices)
            new_vertices.extend(points)
            new_faces.extend([(start,start+1,start+2),(start,start+2,start+3)])
            new_uv.extend(coords)
            tiles += 1
    atlas_path = destination / 'inferred-northern-leaves.png'
    Image.fromarray(atlas).save(atlas_path)
    # Append to a copy of the original mesh: preserve all prior loops, colours,
    # UVs, faces and material slots. No observed geometry is reconstructed.
    crown.data = crown.data.copy()
    material_index = len(crown.data.materials)
    crown.data.materials.append(material(crown.name+' inferred northern cap', atlas_path, False))
    bm = bmesh.new()
    bm.from_mesh(crown.data)
    uv = bm.loops.layers.uv.get('Foliage UV')
    colour = bm.loops.layers.float_color.get('Source ownership')
    if uv is None or colour is None:
        raise ValueError('Missing frozen foliage attributes')
    inverse = crown.matrix_world.inverted()
    vertices = [bm.verts.new(inverse @ Vector(p)) for p in new_vertices]
    for indices in new_faces:
        face = bm.faces.new([vertices[i] for i in indices])
        face.material_index = material_index
        for loop, index in zip(face.loops, indices):
            loop[uv].uv = new_uv[index]
            loop[colour] = (0,1,1,1)
    bm.to_mesh(crown.data)
    bm.free()
    crown.data.update()
    if mesh_prefix(crown.data,*counts) != observed_before:
        raise ValueError('Existing crown geometry, UVs or source ownership changed')
    if edge=='north':assert max(-p[1]*SIN-p[2]*COS for p in new_vertices) < 0
    else:assert min(p[0] for p in new_vertices)>1792
    return dict(source_packet_sha256=sha(packet_path), source_image_sha256=sha(source_path),
        native_edge_span=[left,right], inferred_rise=rise, added_faces=len(new_faces),
        map_edge=edge,
        completion_version='world-aligned-volume-v2',
        added_vertices=len(new_vertices), observed_geometry_preserved=True,
        preserved_crown_prefix_sha256=observed_before,
        method=f'Irregular crossed leaf volume continuing beyond the {edge} image boundary',
        limitation='Missing shape and texture are inferred; native artwork does not establish an exact crown boundary.')


def complete(mask, redo=False):
    worker = OUT / 'forest-v4-round-3/assets' / f'croisement02-tree-{mask:02}'
    receipt = worker / 'inspection/northern-cap-revision.json'
    previous = None
    if receipt.exists() and redo:
        previous = Path(json.loads(receipt.read_text())['previous_worker'])
        receipt.rename(receipt.with_name('northern-cap-archive-'+uuid.uuid4().hex[:8]+'.json'))
    if receipt.exists():
        if json.loads(receipt.read_text())['model_sha256'] != sha(worker/'model.blend'):
            raise ValueError('Northern cap candidate changed')
        return
    old = previous or tree_workspace(mask)
    if old == worker:
        raise ValueError('Expected an earlier frozen worker')
    acquire()
    try:
        old_hash = sha(old/'model.blend')
        report = json.loads((old/'inspection/refinement.json').read_text())
        source_rows = {r['mask']:r['packet'] for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text())}
        packet_path = Path(report.get('source_packet',source_rows[mask]))
        if not (worker/'workspace.json').exists():
            bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            cfg = json.loads((old/'workspace.json').read_text())
            prepare(worker,asset_id=old.name,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],
                source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',
                inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',
                source_mask_manifest=old/'source-masks.json',width=256,height=256,framing_padding=1.4,lighting=cfg['lighting'])
        bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                   if o.type=='MESH' and o.get('asset_group')==old.name]
        crown = next(o for o in objects if o.get('projection_component')=='crown')
        inspection = worker/'inspection'
        inspection.mkdir(exist_ok=True)
        edge='east' if mask==39 else 'north'
        addition = cap(crown,packet_path,mask,inspection,edge)
        saved_meshes = {o.name:o.data.copy() for o in objects}
        for mesh in saved_meshes.values():
            for i, mat in enumerate(mesh.materials):
                mesh.materials[i] = mat.copy()
        bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        modified(worker)
        for obj in objects:
            obj.data = saved_meshes[obj.name]
        validate(worker)
        bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        report.update(model_sha256=sha(worker/'model.blend'),source_packet=str(packet_path),
            status='New boundary-completion candidate; previous approval does not apply to added geometry')
        report['crown']['northern_completion'] = addition
        report['limitations'].append(f'{edge.title()} off-map crown continuation is inferred; the earlier approved worker remains unchanged.')
        write_json(inspection/'refinement.json',report)
        audit(worker)
        render_workspace(worker,256,release_slot=False)
        assert sha(old/'model.blend') == old_hash
        write_json(receipt,dict(model_sha256=report['model_sha256'],previous_worker=str(old),
            previous_model_sha256=old_hash,completion=addition,approval='pending'))
    finally:
        release()


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('masks',type=int,nargs='+')
    parser.add_argument('--redo',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    for mask in args.masks:
        complete(mask,args.redo)
