"""Build one private local wood-field prototype without rendering or remeshing.

Requires an explicitly released model lane. The CPU packet pins all inputs;
this stage creates neutral new surfaces for subsequent source projection and
review. A successful save does not make the result gallery-ready.
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire, release
from refinement_workspace import _geometry


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def protected_state(objects):
    cache = {}
    return {o.name: _geometry(o, protect_appearance=True, appearance_cache=cache) for o in objects}


def split_boundary(bm, original, fractions, candidates):
    """Split real boundary edges; interpolate existing BMesh loop attributes."""
    ordered = []
    for point in original:
        vertex = min(candidates, key=lambda v: (v.co-Vector(point)).length_squared)
        if (vertex.co-Vector(point)).length > .0003:
            raise ValueError('Preserved boundary differs from CPU extraction')
        ordered.append(vertex)
    result = [None]*len(fractions)
    for i in range(len(ordered)):
        start, end = ordered[i], ordered[(i+1) % len(ordered)]
        requests = sorted((float(f), j) for j, (edge, f) in enumerate(fractions) if int(edge) == i)
        previous_fraction = 0.
        current = start
        for fraction, j in requests:
            if fraction < 1e-9:
                result[j] = start
            elif fraction > 1-1e-9:
                result[j] = end
            elif abs(fraction-previous_fraction) < 1e-12:
                result[j] = current
            else:
                edge = bm.edges.get((current, end))
                if edge is None or not edge.is_boundary:
                    raise ValueError('Expected actual open boundary edge')
                _, vertex = bmesh.utils.edge_split(edge, current, (fraction-previous_fraction)/(1-previous_fraction))
                result[j] = vertex
                current, previous_fraction = vertex, fraction
    if any(v is None for v in result):
        raise ValueError('Incomplete boundary subdivision')
    return result


def mesh_report(bm):
    return dict(vertices=len(bm.verts), faces=len(bm.faces),
                nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),
                degenerate_faces=sum(f.calc_area() < 1e-9 for f in bm.faces))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--packet', type=Path, required=True)
    parser.add_argument('--tree', type=int, choices=[32, 38], required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output.exists():
        raise FileExistsError(args.output)
    recipe = json.loads(args.packet.read_text())
    record = next(r for r in recipe['records'] if r['tree'] == args.tree)
    model, payload = Path(record['input_model']), Path(recipe['payload'])
    if sha(model) != record['input_model_sha256'] or sha(payload) != recipe['payload_sha256']:
        raise ValueError('Pinned construction input changed')
    if sha(Path(record['source_mask'])) != record['source_mask_sha256']:
        raise ValueError('Pinned source domain changed')
    arrays = np.load(payload, allow_pickle=False)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(model))
        bpy.context.preferences.filepaths.save_version = 0
        objects = list(bpy.data.collections['Croisement02 Working'].all_objects)
        wood = [o for o in objects if o.type == 'MESH' and o.get('asset_group') == f'croisement02-tree-{args.tree}' and o.get('projection_component') != 'crown']
        primary = next(o for o in wood if o.get('source_node') == record['source_node'])
        protected_objects = [o for o in objects if o.type == 'MESH' and o not in wood]
        protected = protected_state(protected_objects)
        bm = bmesh.new()
        bm.from_mesh(primary.data)
        bmesh.ops.transform(bm, matrix=primary.matrix_world, verts=list(bm.verts))
        z = record['upper_cut_z']
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts)+list(bm.edges)+list(bm.faces), dist=1e-6,
                              plane_co=(0, 0, z), plane_no=(0, 0, 1), clear_inner=True)
        retained_vertices = list(bm.verts)
        retained_positions = [(v, v.co.copy()) for v in retained_vertices]
        upper_boundary = [v for v in bm.verts if abs(v.co.z-z) < .0003 and v.is_boundary]
        material_count = len(primary.data.materials)
        prefix = f'tree{args.tree}'
        field_vertices = [bm.verts.new(p) for p in arrays[prefix+'_vertices']]
        for triangle in arrays[prefix+'_faces']:
            face = bm.faces.new([field_vertices[int(i)] for i in triangle])
            face.material_index = material_count
            face.smooth = True
        for collar in record['collars']:
            key = collar['array_prefix']
            low = split_boundary(bm, arrays[key+'_lower_original'], arrays[key+'_lower_edge_positions'], field_vertices)
            high = split_boundary(bm, arrays[key+'_upper_original'], arrays[key+'_upper_edge_positions'], upper_boundary)
            rows = arrays[key+'_rows']
            parameters = arrays[key+'_parameters']
            # Endpoint-refined samples validate the continuous mapping. Uniform
            # construction rows avoid retaining microscopic diagnostic triangles.
            targets = np.unique(np.r_[np.linspace(0,1,49),.001,.002,.005,.01,.99,.995,.998,.999])
            chosen = sorted(set(int(np.argmin(abs(parameters-t))) for t in targets))
            rings = [low]+[[bm.verts.new(p) for p in rows[i]] for i in chosen[1:-1]]+[high]
            for a, b in zip(rings, rings[1:]):
                for j in range(len(a)):
                    k = (j+1) % len(a)
                    for verts in [(a[j], a[k], b[k]), (a[j], b[k], b[j])]:
                        face = bm.faces.new(verts)
                        face.material_index = material_count
                        face.smooth = True
        drift = max((v.co-point).length for v, point in retained_positions)
        if drift != 0.:
            raise ValueError('Retained upper vertex moved')
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        report = mesh_report(bm)
        if report['nonmanifold_edges'] or report['degenerate_faces']:
            raise ValueError(report)
        # Keep a continuous diagnostic owner until native-owner splitting and
        # source projection can be performed and audited together. This model
        # cannot be substituted for a finished refinement workspace.
        mesh = bpy.data.meshes.new(f'Private continuous field wood{args.tree}')
        bm.to_mesh(mesh)
        bm.free()
        for material in primary.data.materials:
            mesh.materials.append(material)
        neutral = bpy.data.materials.new(f'Unprojected field wood{args.tree}')
        neutral.diffuse_color = (.4, .4, .4, 1.)
        mesh.materials.append(neutral)
        primary.data = mesh
        primary.parent = None
        primary.matrix_world = Matrix.Identity(4)
        for obj in wood:
            if obj != primary:
                bpy.data.objects.remove(obj, do_unlink=True)
        if protected != protected_state(protected_objects):
            raise ValueError('Protected crown or other asset changed')
        args.output.mkdir(parents=True)
        destination = args.output/'model.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(destination))
        if sha(model) != record['input_model_sha256']:
            raise ValueError('Input changed')
        evidence = dict(status='PRIVATE HOLD: neutral local geometry prototype only', model_sha256=sha(destination), input_model=str(model), input_model_sha256=record['input_model_sha256'], packet=str(args.packet.resolve()), packet_sha256=sha(args.packet), recipe_sha256=sha(Path(__file__)), mesh=report, retained_vertex_max_drift=drift, protected=protected, source_mask_unchanged=True,
                        pending=['Native owner32 partition and source projection', 'Nine disconnected native32 samples where applicable', 'Saved source and opaque-ground coverage', 'Actual and solid eight-view independent review'], gallery_ready=False)
        (args.output/'evidence.json').write_text(json.dumps(evidence, indent=2)+'\n')
        print(destination)
    finally:
        release()


if __name__ == '__main__':
    main()
