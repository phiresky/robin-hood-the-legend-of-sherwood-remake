"""Small observed foliage fragments on an irregular full-depth crown envelope."""
import math
import numpy as np
from mathutils import Vector
from rounded_interior_geometry import build as volume
from tree_geometry import SIN, COS, RAY, replace_mesh


def build(obj, packet, ground_y):
    report = volume(obj, packet, ground_y)
    mesh = obj.data
    fx, fy, fw, fh = packet['bbox']
    cx, cy = int(fx + fw / 2), fy + fh / 2
    rx = fw * .5 + 12
    radii = np.array([rx, rx * 1.22, max(fh * .56, rx * .70)])
    center = np.array([float(cx), -ground_y / SIN, (ground_y - cy) / COS])
    ray = np.asarray(RAY)
    if center[2] - radii[2] < 50:
        center += ray * ((50 + radii[2] - center[2]) / SIN)
    a = np.sum((ray / radii) ** 2)
    vertices, faces, uvs, slots, known = [], [], [], [], []
    seen = set()
    uv_layer = mesh.uv_layers['Foliage UV']
    for face in mesh.polygons:
        coords = [tuple(uv_layer.data[i].uv) for i in face.loop_indices]
        observed_skin = face.material_index in (0, 2)
        if observed_skin:
            # The earlier volume builder repeats this exact source tile three
            # times at different depths. A curved skin needs only one copy.
            key = (face.material_index, tuple(coords))
            if key in seen:
                continue
            seen.add(key)
        original = [np.asarray(obj.matrix_world @ mesh.vertices[mesh.loops[loop].vertex_index].co)
                    for loop in face.loop_indices]
        patches = [(original, coords)]
        if observed_skin:
            # Small independent fragments sample the curved envelope without
            # stretching source leaves across steeply sloping shell faces.
            n = 4
            def sample(i, j):
                weights = np.array([1-(i+j)/n, i/n, j/n])
                return (sum(weights[k]*original[k] for k in range(3)),
                        sum(weights[k]*np.asarray(coords[k]) for k in range(3)))
            patches = []
            for i in range(n):
                for j in range(n-i):
                    indices = [(i,j),(i+1,j),(i,j+1)]
                    triangles = [indices]
                    if i+j < n-1:
                        triangles.append([(i+1,j),(i+1,j+1),(i,j+1)])
                    for indices in triangles:
                        samples = [sample(*index) for index in indices]
                        patches.append(([v[0] for v in samples],[v[1] for v in samples]))
        for points, patch_uv in patches:
            start = len(vertices)
            depth = None
            if observed_skin:
                average = np.mean(points,axis=0)
                x,y = average[0], -average[1]*SIN-average[2]*COS
                relative = np.array([x-cx,-(y-cy)*SIN,-(y-cy)*COS])
                b = 2*np.sum(relative*ray/radii**2)
                c = np.sum((relative/radii)**2)-1
                depth = (-b+math.sqrt(max(0.,b*b-4*a*c)))/(2*a)
                depth += 5*math.sin(x*.052+y*.031)+3*math.sin(x*.11-y*.057)
                depth += 1.5*math.sin(x*2.731+y*3.237)
                if face.material_index == 2:
                    depth -= .02
            for point, uv in zip(points,patch_uv):
                if observed_skin:
                    x,y = point[0], -point[1]*SIN-point[2]*COS
                    relative = np.array([x-cx,-(y-cy)*SIN,-(y-cy)*COS])
                    point = center+relative+ray*depth
                vertices.append(point.tolist())
                uvs.append(tuple(uv))
            faces.append(tuple(range(start,len(vertices))))
            slots.append(face.material_index)
            known.append(face.material_index == 0)
    mesh_report = replace_mesh(obj,vertices,faces,uvs,list(mesh.materials),slots,known)
    xyz = np.asarray(vertices)
    result = dict(report, **mesh_report)
    result.pop('leaf_clusters', None)
    result['observed_fragment_triangles'] = sum(known)
    result.update(vertices=len(vertices),faces=len(faces),width=float(np.ptp(xyz[:,0])),depth=float(np.ptp(xyz[:,1])),
        geometry_version='microfragment-curved-envelope-irregular-volume-v2',
        method='Small source-facing fragments sample an irregular curved envelope, with paired inferred backs and randomly rotated interior leaf clusters',
        removed_repeated_observed_layers=True)
    return result
