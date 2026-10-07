"""Bounded private source projection and uncapped native-owner partition.

No rendering or canonical writes. Missing retained source islands fail before
model mutation. Run only with a released Blender lane and two threads.
"""
import argparse, hashlib, json, sys
from pathlib import Path
import bpy, bmesh, numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire, release
from refinement_workspace import _geometry
from source_projection_bake import bake
from wood_source_partition import projected, source_pixel_fragment, owner32, RAY, SIN, support_sections


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def protected_state(objects):
    cache = {}
    return {o.name: _geometry(o, protect_appearance=True, appearance_cache=cache) for o in objects}


def faces_hash(objects):
    faces = []
    for obj in objects:
        points = [tuple(obj.matrix_world@v.co) for v in obj.data.vertices]
        for face in obj.data.polygons:
            faces.append(tuple(sorted(points[i] for i in face.vertices)))
    return hashlib.sha256(repr(sorted(faces)).encode()).hexdigest()


def old_islands(model, tree, pixels):
    bpy.ops.wm.open_mainfile(filepath=str(model))
    objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type == 'MESH' and o.get('asset_group') == f'croisement02-tree-{tree}' and o.get('projection_component') != 'crown']
    properties = {o['source_node']: dict(o.items()) for o in objects}
    triangles = []
    for obj in objects:
        obj.data.calc_loop_triangles()
        p = np.asarray([obj.matrix_world@v.co for v in obj.data.vertices])
        triangles.extend(p[list(t.vertices)] for t in obj.data.loop_triangles)
    triangles = np.asarray(triangles)
    if not pixels:
        return [], properties, []
    bvh = BVHTree.FromPolygons([Vector(p) for p in triangles.reshape(-1, 3)], np.arange(len(triangles)*3).reshape(-1, 3).tolist(), all_triangles=True)
    fragments, checks = [], []
    q = projected(triangles.reshape(-1, 3)).reshape(-1, 3, 2)
    for x, y in pixels:
        origin = Vector((x+.5, -(y+.5)/SIN, 0))+Vector(RAY)*5000
        hit = bvh.ray_cast(origin, -Vector(RAY))
        checks.append(dict(pixel=[x,y], old_surface_hit=list(hit[0]) if hit[0] is not None else None))
        if hit[0] is None:
            raise ValueError(f'Native pixel {x},{y} has no retained old wood surface; source role/depth review required')
        selected = np.where((q[:,:,0].max(axis=1)>=x)&(q[:,:,0].min(axis=1)<=x+1)&(q[:,:,1].max(axis=1)>=y)&(q[:,:,1].min(axis=1)<=y+1))[0]
        for index in selected:
            triangle = triangles[index]
            if np.dot(np.cross(triangle[1]-triangle[0], triangle[2]-triangle[0]), RAY) <= 1e-9:
                continue
            fragment = source_pixel_fragment(triangle, [x,y])
            if len(fragment) >= 3:
                fragments.append(fragment)
    return fragments, properties, checks


def partition(primary, tree, properties):
    mesh = primary.data
    points = [v.co.copy() for v in mesh.vertices]
    normals = [v.normal.copy() for v in mesh.vertices]
    lookup = KDTree(len(points))
    for i,p in enumerate(points): lookup.insert(p,i)
    lookup.balance()
    labels = [owner32(face.center) if tree == 32 else 94 for face in mesh.polygons]
    results = []
    for owner in sorted(set(labels)):
        obj = primary.copy()
        obj.data = mesh.copy()
        obj.name = f'Private field wood owner{owner}'
        bpy.data.collections['Croisement02 Working'].objects.link(obj)
        for key,value in properties[f'building-{owner:03d}'].items(): obj[key] = value
        bm = bmesh.new(); bm.from_mesh(obj.data); bm.faces.ensure_lookup_table()
        bmesh.ops.delete(bm, geom=[face for i,face in enumerate(bm.faces) if labels[i] != owner], context='FACES')
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
        bm.to_mesh(obj.data); bm.free()
        obj.data.normals_split_custom_set_from_vertices([normals[lookup.find(v.co)[1]] for v in obj.data.vertices])
        results.append(obj)
    return results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--tree', type=int, choices=[32,38], required=True)
    parser.add_argument('--prototype', type=Path, required=True)
    parser.add_argument('--packet', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--domain-review', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output.exists(): raise FileExistsError(args.output)
    evidence = json.loads((args.prototype/'evidence.json').read_text())
    packet = json.loads(args.packet.read_text())
    record = next(r for r in packet['records'] if r['tree'] == args.tree)
    model = args.prototype/'model.blend'; old = Path(record['input_model'])
    if sha(model) != evidence['model_sha256'] or sha(old) != record['input_model_sha256']: raise ValueError('Pinned model changed')
    if sha(Path(record['source_mask'])) != record['source_mask_sha256']: raise ValueError('Source mask changed')
    remaining_pixels = record['unresolved_source_coordinates']
    source_manifest = old.parent/'source-masks.json'
    domain_review = None
    if args.domain_review:
        domain_review = json.loads(args.domain_review.read_text())
        proposal_path = Path(domain_review['proposal']) if domain_review.get('proposal') else args.domain_review.parent/'proposal.json'
        if domain_review.get('status') not in ('accepted','PASS_ROOT_SCOPED_WOOD_DOMAIN_CORRECTION') or sha(proposal_path) != domain_review['proposal_sha256']: raise ValueError('Unaccepted or stale scoped domain review')
        proposal = json.loads(proposal_path.read_text())
        if args.tree != 32 or sorted(proposal['changed_pixels']) != sorted(remaining_pixels): raise ValueError('Domain correction does not exactly cover the unresolved pixels')
        if sha(Path(proposal['first_hit_evidence'])) != proposal['first_hit_evidence_sha256'] or sha(Path(proposal['proposed_mask'])) != proposal['proposed_mask_sha256']: raise ValueError('Scoped domain evidence changed')
        for proof in proposal['proof_inputs']:
            if sha(Path(proof['path'])) != proof['sha256']: raise ValueError('Reviewed nonwood receiver changed')
        remaining_pixels = []
        source_manifest = proposal_path.parent/'source-masks.json'
    args.output.mkdir(parents=True)
    if domain_review:
        revised = json.loads(source_manifest.read_text())
        for assignment in revised['projections']['exterior']['assignments']:
            if assignment.get('asset_group') == 'croisement02-tree-32' and assignment.get('mask_indices') == [3801]:
                assignment['reviewed'] = True
                assignment['review_note'] = 'Exact nine-pixel nonwood ownership correction independently reviewed; crown131 and shrub71 retain those observed pixels.'
        source_manifest = args.output/'source-masks.json'
        source_manifest.write_text(json.dumps(revised,indent=2)+'\n')
    acquire()
    try:
        receiver_path = ROOT/'level-editor/work/croisement02-refinement/ground-receiver-review-v5/model.blend'
        receiver_hash = 'efed22896d8c4075bb0ed346bf6a36234d28762574b5b083c546b23d09c0c0bf'
        if sha(receiver_path) != receiver_hash: raise ValueError('Pinned actual ground receiver changed')
        bpy.ops.wm.open_mainfile(filepath=str(receiver_path))
        receivers = [o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type == 'MESH' and o.get('source_node') == 'ground']
        if len(receivers) != 1: raise ValueError('Ambiguous actual ground receiver')
        receiver = receivers[0]; receiver.data.calc_loop_triangles()
        ground_points = [receiver.matrix_world@v.co for v in receiver.data.vertices]
        ground_faces = [list(t.vertices) for t in receiver.data.loop_triangles]
        if len(ground_points) != 4 or len(ground_faces) != 2: raise ValueError('Unexpected ground geometry')
        ground_bvh = BVHTree.FromPolygons(ground_points, ground_faces, all_triangles=True)
        ground_evidence = dict(model=str(receiver_path), model_sha256=receiver_hash, vertices=[list(p) for p in ground_points], triangles=ground_faces)
        fragments, properties, pixel_checks = old_islands(old, args.tree, remaining_pixels)
        bpy.ops.wm.open_mainfile(filepath=str(model)); bpy.context.preferences.filepaths.save_version = 0
        collection = bpy.data.collections['Croisement02 Working']
        primary = next(o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == f'croisement02-tree-{args.tree}' and o.get('projection_component') != 'crown')
        protected_objects = [o for o in collection.all_objects if o.type == 'MESH' and o != primary]
        protected = protected_state(protected_objects)
        body_hash = faces_hash([primary])
        # Partition the existing body first: no cap, overlap, vertex displacement,
        # or polygon shape change is introduced by the source-owner labels.
        parts = partition(primary, args.tree, properties)
        if faces_hash(parts) != body_hash: raise ValueError('Ownership partition changed world faces')
        bpy.data.objects.remove(primary, do_unlink=True)
        for obj in parts:
            owned = [f for f in fragments if owner32(np.mean(f,axis=0)) == int(obj['source_node'].split('-')[-1])]
            if not owned: continue
            bm = bmesh.new(); bm.from_mesh(obj.data)
            layer = bm.faces.layers.int.get('retained_source_island') or bm.faces.layers.int.new('retained_source_island')
            for fragment in owned:
                verts = [bm.verts.new(v) for v in fragment]
                for i in range(1,len(verts)-1):
                    if (verts[i].co-verts[0].co).cross(verts[i+1].co-verts[0].co).length < 2e-9: continue
                    face = bm.faces.new((verts[0],verts[i],verts[i+1])); face.material_index = len(obj.data.materials)-1; face[layer] = 1
            bm.to_mesh(obj.data); bm.free()
        before_projection = faces_hash(parts)
        scoped = {o.name:[f.index for f in o.data.polygons if o.data.materials[f.material_index].name.startswith('Unprojected field wood')] for o in parts}
        if any(not faces for faces in scoped.values()): raise ValueError('Empty new-surface projection scope')
        cfg = json.loads((old.parent/'workspace.json').read_text())
        projection = bake(cfg['map_name'], cfg['source_path'], args.output/'projection.json', receiver_object_names=[o.name for o in parts], receiver_face_indices={name:set(faces) for name,faces in scoped.items()}, material_suffix='local-field-only', projection_label='exterior', elevation_deg=35., preserve_authored=False, source_mask_manifest=str(source_manifest))
        if faces_hash(parts) != before_projection: raise ValueError('Projection changed world faces')
        if protected != protected_state(protected_objects): raise ValueError('Projection changed protected assets')
        support=[]
        for obj in parts:
            obj.data.calc_loop_triangles()
            p=np.asarray([obj.matrix_world@v.co for v in obj.data.vertices]); t=np.asarray([list(f.vertices) for f in obj.data.loop_triangles])
            near = p[(p[:,2]>=0)&(p[:,2]<=.6)]
            contacts = []
            for point in near:
                hit = ground_bvh.ray_cast(Vector((point[0],point[1],100)),Vector((0,0,-1)))
                if hit[0] is None: raise ValueError('Basal wood lies outside actual ground footprint')
                contacts.append(float(point[2]-hit[0].z))
            support.append(dict(source_node=obj['source_node'],ground=support_sections(p,t),near_ground=support_sections(p,t,.5),actual_receiver_samples=len(contacts),minimum_vertical_gap=min(contacts) if contacts else None,maximum_vertical_gap=max(contacts) if contacts else None))
        bpy.ops.wm.save_as_mainfile(filepath=str(args.output/'model.blend'))
        if sha(model) != evidence['model_sha256'] or sha(old) != record['input_model_sha256']: raise ValueError('Input mutation')
        report=dict(status='PRIVATE HOLD: projected prototype, no renders or approval', model_sha256=sha(args.output/'model.blend'), previous_prototype=str(args.prototype), previous_model_sha256=evidence['model_sha256'], packet_sha256=sha(args.packet), body_world_faces_sha256=body_hash, partition_preserves_world_faces=True, internal_caps_added=0, domain_review=str(args.domain_review) if args.domain_review else None, domain_review_sha256=sha(args.domain_review) if args.domain_review else None, disconnected_source_checks=pixel_checks, retained_source_fragments=len(fragments), source_scoped_projection=scoped, support=support, actual_ground_receiver=ground_evidence, protected=protected, pending=['Opaque source coverage and nine-pixel exact coverage','Actual terrain receiver footprint and38 terrain-occluded source coverage','Solid/actual independent visual review'])
        (args.output/'evidence.json').write_text(json.dumps(report,indent=2)+'\n')
    except Exception as error:
        (args.output/'failure.json').write_text(json.dumps(dict(status='PRIVATE HOLD; failed before readiness',error=str(error)),indent=2)+'\n')
        raise
    finally: release()


if __name__ == '__main__': main()
