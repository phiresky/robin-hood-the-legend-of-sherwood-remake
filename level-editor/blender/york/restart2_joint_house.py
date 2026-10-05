"""Private paired-house hypothesis: separate roof shells from recessed walls."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
HOUSE = 'york-southwest-square-west-house'
BAY = 'york-market-southeast-tall-narrow-house'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', required=True)
    parser.add_argument('--setback', type=float, default=.1)
    parser.add_argument('--bay-notch', action='store_true')
    parser.add_argument('--bay-outline', action='store_true')
    parser.add_argument('--notch-intercept', type=float, default=1153)
    parser.add_argument('--retain-upper-gable', action='store_true')
    parser.add_argument('--gable-eave', action='store_true')
    parser.add_argument('--extend-left-roof', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    destination = OUT / 'restart2' / args.version
    if destination.exists():
        raise FileExistsError(destination)
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
    from freeze_tooling import select_tooling
    select_tooling(json.loads((OUT / 'tooling/current.json').read_text())['directory'])
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import bpy
    import bmesh
    from mathutils import Matrix
    from refinement_workspace import prepare
    from inspect_geometry import geometric_first_hits
    destination.mkdir(parents=True)
    source = OUT / 'geometry-pass-01/assets' / HOUSE / 'model.blend'
    bpy.ops.wm.open_mainfile(filepath=str(source))
    collection = bpy.data.collections['york Working']
    objects = [o for o in collection.all_objects if o.type == 'MESH' and not o.hide_render]

    def fingerprint(obj):
        data = {'vertices': [list(obj.matrix_world @ v.co) for v in obj.data.vertices],
                'faces': [list(p.vertices) for p in obj.data.polygons],
                'uv': [[list(d.uv) for d in layer.data] for layer in obj.data.uv_layers]}
        return hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()

    outside = {o.name: fingerprint(o) for o in objects if o.get('asset_group') not in (HOUSE, BAY)}
    changes = []
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def subtract_bay_recess(vertices, faces):
        meshes, temporary = [], []
        def object_for(name, points, polygons):
            mesh = bpy.data.meshes.new(name)
            mesh.from_pydata(points, [], polygons)
            mesh.update()
            bm = bmesh.new()
            bm.from_mesh(mesh)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            bm.to_mesh(mesh)
            bm.free()
            obj = bpy.data.objects.new(name, mesh)
            bpy.context.scene.collection.objects.link(obj)
            meshes.append(mesh)
            temporary.append(obj)
            return obj
        body = object_for('Private body boolean', vertices, faces)
        footprint = [(500,250+args.notch_intercept), (646,323+args.notch_intercept), (646,1600), (500,1600)]
        # The native dark gable above the attached roof survives the lower
        # recess. Its lower edge projects to source row 1228 on the original
        # front datum. This upper cantilever remains an explicit hypothesis.
        cutter_vertices = [(x,-y/sine,80/cosine) for x,y in footprint]
        cutter_vertices += [(x,-y/sine,(.475*x-28 if args.retain_upper_gable else 400)/cosine)
                            for x,y in footprint]
        cutter_faces = [(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
        cutter = object_for('Private attached bay recess', cutter_vertices, cutter_faces)
        modifier = body.modifiers.new('Attached bay recess', 'BOOLEAN')
        modifier.operation = 'DIFFERENCE'
        modifier.solver = 'EXACT'
        modifier.object = cutter
        bpy.context.view_layer.objects.active = body
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        result = ([tuple(v.co) for v in body.data.vertices], [tuple(p.vertices) for p in body.data.polygons])
        for obj in temporary:
            bpy.data.objects.remove(obj, do_unlink=True)
        for mesh in meshes:
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
        return result

    def replace(obj, vertices, faces):
        before = fingerprint(obj)
        materials = list(obj.data.materials)
        mesh = bpy.data.meshes.new(obj.name + ' reconstructed')
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.00001)
        bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=.00001)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        if any(not e.is_manifold for e in bm.edges):
            (destination / 'topology-failure.json').write_text(json.dumps({
                'object': obj.name,
                'edges': [{'vertices': [list(v.co) for v in e.verts], 'faces': len(e.link_faces)}
                          for e in bm.edges if not e.is_manifold]}, indent=2) + '\n')
            raise ValueError('Open reconstructed mesh: ' + obj.name)
        bm.to_mesh(mesh)
        bm.free()
        obj.data = mesh
        obj.parent = None
        obj.matrix_world = Matrix.Identity(4)
        for material in materials:
            mesh.materials.append(material)
        mesh.uv_layers.new(name='UVMap')
        changes.append({'object': obj.name, 'source_node': obj['source_node'],
                        'before': before, 'after': fingerprint(obj),
                        'vertices': len(mesh.vertices), 'faces': len(mesh.polygons)})

    for node in ('building-309', 'building-310', 'building-311'):
        matches = [o for o in objects if o.get('asset_group') == HOUSE and o.get('source_node') == node]
        if len(matches) != 1:
            raise ValueError('Ambiguous roof part ' + node)
        obj = matches[0]
        mesh = obj.data
        # The retained roof tops are observed. Their earlier downward extrusion
        # was an occlusion proxy and is not evidence for full-height walls.
        top_faces = [p for p in mesh.polygons if (obj.matrix_world.to_3x3() @ p.normal).z > .2
                     and min((obj.matrix_world @ mesh.vertices[i].co).z * cosine for i in p.vertices) > 200]
        if not top_faces:
            raise ValueError('Missing native roof top ' + node)
        top = bmesh.new()
        top.from_mesh(mesh)
        keep = {p.index for p in top_faces}
        top.faces.ensure_lookup_table()
        bmesh.ops.delete(top, geom=[f for f in top.faces if f.index not in keep], context='FACES')
        bmesh.ops.remove_doubles(top, verts=list(top.verts), dist=.0001)
        top.verts.ensure_lookup_table()
        top.verts.index_update()
        vertices = [tuple(obj.matrix_world @ v.co) for v in top.verts]
        top_indices = [tuple(v.index for v in face.verts) for face in top.faces]
        top.free()
        if node=='building-311' and args.extend_left_roof:
            # Continue the entire low roof edge to the observed projecting
            # eave, rather than attaching an unsupported narrow timber.
            vertices=[(x-15.5,y+7.34/sine,z-13.353/cosine) if z*cosine<250 else (x,y,z)
                      for x,y,z in vertices]
        n = len(vertices)
        vertices += [(x, y, z-2.5/cosine) for x, y, z in vertices]
        faces = top_indices
        boundary = {}
        for face in faces:
            for a, b in zip(face, face[1:] + face[:1]):
                key = tuple(sorted((a, b)))
                if key in boundary:
                    del boundary[key]
                else:
                    boundary[key] = (a, b)
        faces += [tuple(i+n for i in reversed(face)) for face in faces.copy()]
        faces += [(a, a+n, b+n, b) for a, b in boundary.values()]
        if node == 'building-309':
            # Hidden body recession is an explicit hypothesis. Follow the roof
            # longitudinal axis so its gable stays under the retained planes.
            profile = [(580.5,1475.9,90.00101), (668.5,1517.7,90.00101),
                       (668.5,1517.7,235.5), (616.5,1492.7,284.3),
                       (593.9,1482.2,260.4), (580.5,1475.9,245.8)]
            body_vertices = []
            for t in (args.setback, .97):
                body_vertices += [(x+95.4*t, -(y-65.0*t)/sine, z/cosine) for x,y,z in profile]
            count = len(profile)
            body_faces = [tuple(reversed(range(count))), tuple(count+i for i in range(count))]
            for i in range(count):
                k = (i+1) % count
                body_faces.append((i,k,count+k,count+i))
            if args.bay_notch:
                body_vertices, body_faces = subtract_bay_recess(body_vertices, body_faces)
            start = len(vertices)
            vertices += body_vertices
            faces += [tuple(start+i for i in face) for face in body_faces]
            if args.gable_eave:
                # The painted projecting timber lies beyond the coarse roof
                # proxy. Its two source anchors are (565,1231) and (594,1222).
                # Depth follows the existing front gable plane; section and
                # hidden end join remain inferred.
                from mathutils import Vector
                def timber(a,b,width,depth):
                    a=Vector((a[0],-a[1]/sine,a[2]/cosine))
                    b=Vector((b[0],-b[1]/sine,b[2]/cosine))
                    axis=(b-a).normalized()
                    transverse=axis.cross(Vector((0,1,0))).normalized()*width/2
                    normal=axis.cross(transverse).normalized()*depth/2
                    start=len(vertices)
                    for point in (a,b):
                        vertices.extend(tuple(point+s*transverse+t*normal)
                                        for s,t in ((-1,-1),(1,-1),(1,1),(-1,1)))
                    faces.extend(tuple(start+i for i in f) for f in
                                 ((3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)))
                timber((565,1468.5,237.5),(594,1482.5,260.5),3,3)
                timber((578.5,1474.9,226),(578.5,1474.9,244.9),3,3)
        replace(obj, vertices, faces)

    bay_source = OUT / 'geometry-pass-01/assets' / BAY / 'model.blend'
    with bpy.data.libraries.load(str(bay_source), link=False) as (available, imported):
        imported.objects = available.objects
    imported_objects = [o for o in imported.objects if o]
    candidates = [o for o in imported_objects if o.type == 'MESH' and o.get('asset_group') == BAY and not o.hide_render]
    if len(candidates) != 1:
        raise ValueError('Ambiguous bay candidate')
    donor = candidates[0]
    target = next(o for o in objects if o.get('asset_group') == BAY)
    before = fingerprint(target)
    target.data = donor.data.copy()
    target.parent = None
    target.matrix_world = donor.matrix_world.copy()
    changes.append({'object': target.name, 'source_node': target['source_node'],
                    'before': before, 'after': fingerprint(target), 'source': str(bay_source)})
    for obj in imported_objects:
        bpy.data.objects.remove(obj, do_unlink=True)
    if args.bay_outline:
        import numpy as np
        vertices, faces = [], []
        def ring(points, heights):
            start = len(vertices)
            vertices.extend((x,-y/sine,z/cosine) for (x,y),z in zip(points,heights))
            return list(range(start,len(vertices)))
        def join(a,b):
            for i in range(len(a)):
                k=(i+1)%len(a)
                faces.append((a[i],a[k],b[k],b[i]))
        lower = [(584,1473),(623,1492.5),(646,1477),(605,1457.5)]
        upper = [(574,1469.5),(623,1492.5),(646,1477),(597,1454)]
        a=ring(lower,[90.00101]*4)
        b=ring(lower,[129.5]*4)
        c=ring(upper,[129.5]*4)
        d=ring(upper,[197.5,197.5,220.5,220.5])
        faces.append(tuple(reversed(a)))
        join(a,b);join(b,c);join(c,d)
        faces.append(tuple(d))
        # Source silhouette has a clipped rear junction below the larger roof,
        # not the first approximation's simple quadrilateral. Keep the same
        # physical roof plane while fitting that observed junction outline.
        contour=[(569,1268),(622,1292),(647,1252),(630,1234),(617,1228),(591,1228)]
        coefficients=np.linalg.solve(np.array([[573,1468,1],[622,1492,1],[644,1477,1]],dtype=float),[200,200,223])
        aa,bb,cc=coefficients
        heights=[float((aa*x+bb*y+cc)/(1-bb)) for x,y in contour]
        points=[(x,y+z) for (x,y),z in zip(contour,heights)]
        low=ring(points,[z-2.5 for z in heights])
        high=ring(points,heights)
        faces.extend([tuple(reversed(low)),tuple(high)])
        join(low,high)
        replace(target,vertices,faces)
        (destination/'bay-outline-observations.json').write_text(json.dumps({
            'source_pixels':contour,'basis':'Native green roof edge inspected against source artwork and mask 217. Corners carry 1–3 pixel uncertainty.',
            'roof_plane_game_coefficients':list(coefficients),'body_east_edge_x':646,
            'scope':'Geometry hypothesis, not source coverage approval.'},indent=2)+'\n')
    if outside != {o.name: fingerprint(o) for o in objects if o.get('asset_group') not in (HOUSE, BAY)}:
        raise ValueError('Outside paired-house geometry or UV changed')
    scene = bpy.data.scenes['york Refinement']
    bpy.context.window.scene = scene
    bpy.context.preferences.filepaths.save_version = 0
    model = destination / 'pair.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(model), compress=True)
    geometric_first_hits(scene, BAY, [545,1200,680,1430], destination)
    report = {'status': 'private geometric hypothesis; not approved', 'setback_fraction': args.setback,
              'bay_notch': args.bay_notch,
              'bay_outline_revision': args.bay_outline,
              'notch_intercept': args.notch_intercept,
              'retain_upper_gable': args.retain_upper_gable,
              'gable_eave': args.gable_eave,
              'extend_left_roof': args.extend_left_roof,
              'changes': changes, 'outside_meshes_preserved': len(outside),
              'inferred': ['Main gable body recessed below unchanged roof tops.',
                           'Roof underside thickness 2.5 native height units; body and roof are separate closed volumes.'],
              'model_sha256': hashlib.sha256(model.read_bytes()).hexdigest()}
    (destination / 'geometry.json').write_text(json.dumps(report, indent=2) + '\n')
    for asset in (BAY, HOUSE):
        bpy.ops.wm.open_mainfile(filepath=str(model))
        prepare(destination / 'assets' / asset, asset_id=asset,
                scene_name='york Refinement', collection_name='york Working',
                source_path=OUT / 'baseline/covered.png',
                grouping_manifest=ROOT / 'level-editor/refinement/catalogs/york.json',
                inventory_path=OUT / 'inventory/inventory.json',
                review_path=OUT / 'geometry-pass-01/grouping-reconciliation.json')


if __name__ == '__main__':
    main()
