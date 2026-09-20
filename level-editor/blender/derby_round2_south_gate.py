"""Round south gate roof surfaces using reviewed source silhouette profiles."""
import math
import bpy
import bmesh
from mathutils import Vector

TAG = 'south-gate-round2-radial-v1'


def round_drum(obj, cx, cy, profile):
    """Replace collision facets with the source's concentric masonry rings."""
    vertices, faces = [], []
    count = 96
    for z, r in profile:
        vertices.extend((cx+r*math.cos(2*math.pi*j/count),
                         cy+r*math.sin(2*math.pi*j/count),z) for j in range(count))
    faces.append(tuple(reversed(range(count))))
    for k in range(len(profile)-1):
        faces.extend((k*count+j,k*count+(j+1)%count,(k+1)*count+(j+1)%count,
                      (k+1)*count+j) for j in range(count))
    faces.append(tuple((len(profile)-1)*count+j for j in range(count)))
    mesh=bpy.data.meshes.new(obj.name+' / round masonry')
    inverse=obj.matrix_world.inverted()
    mesh.from_pydata([inverse@Vector(p) for p in vertices],[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges) or any(f.calc_area()<1e-8 for f in bm.faces):
        raise ValueError('Invalid round masonry')
    bm.to_mesh(mesh);bm.free()
    mesh.materials.append(bpy.data.materials['Round two south gate neutral'])
    uv=mesh.uv_layers.new(name='Source projection')
    for loop in mesh.loops:
        p=obj.matrix_world@mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(p.x/1920,1-(-p.y*math.sin(math.radians(35))-p.z*math.cos(math.radians(35)))/2752)
    obj.data=mesh
    obj['round2_south_gate_drum']=TAG


def refine():
    working = bpy.data.collections['Derby Working']
    active = {o.get('source_node'): o for o in working.objects
              if o.type == 'MESH' and not o.hide_render}
    reports = []
    for name, ids, cx, cy, radius, power, tip in (
            ('west', range(17, 23), 754.3654, -4256.46, 52.7554, 1.5326, 2128.0),
            ('east', range(13, 17), 942.7955, -4176.46, 49.3236, 1.3485, 2088.0)):
        sources = [active[f'building-{i:03d}'] for i in ids]
        if all(o.get('round2_south_gate') == TAG for o in sources):
            reports.append({'tower': name, 'reused': True})
            continue
        sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
        base = 263.0006
        top = (-cy*sine-tip)/cosine
        # Continuous round sections replace the former independently clamped wedges.
        # The narrow metal tip and rolled eave are visible in the source crop.
        profiles = [(base-4, radius-3), (base-2, radius), (base, radius)]
        profiles += [(base+(top-base)*t, radius*(1-t)**power)
                     for t in [i/24 for i in range(1,24)]]
        profiles += [(top, 1.4), (top+10, .8), (top+14, 0)]
        directions = []
        for o in sources:
            pts = [o.matrix_world@v.co for v in o.data.vertices]
            low = [p for p in pts if p.z < base+2 and (p.x-cx)**2+(p.y-cy)**2 > 100]
            average = sum(low, Vector())/len(low)
            directions.append(math.atan2(average.y-cy, average.x-cx))
        allocations = {o: [] for o in sources}
        count = 96
        for j in range(count):
            angle = 2*math.pi*(j+.5)/count
            owner = min(range(len(sources)), key=lambda k:
                        abs(math.atan2(math.sin(angle-directions[k]), math.cos(angle-directions[k]))))
            allocations[sources[owner]].append(j)
        for o in sources:
            vertices, faces = [], []
            for j in allocations[o]:
                start = len(vertices)
                for z, r in profiles[:-1]:
                    for angle in (2*math.pi*j/count, 2*math.pi*(j+1)/count):
                        vertices.append((cx+r*math.cos(angle), cy+r*math.sin(angle), z))
                n = len(profiles)-1
                apex = len(vertices); vertices.append((cx, cy, profiles[-1][0]))
                center = len(vertices); vertices.append((cx, cy, profiles[0][0]))
                faces += [(start+2*k, start+2*k+1, start+2*k+3, start+2*k+2) for k in range(n-1)]
                faces += [(start+2*n-2, start+2*n-1, apex),
                          tuple([center]+[start+2*k for k in range(n)]+[apex]),
                          tuple([center,apex]+[start+2*k+1 for k in reversed(range(n))]),
                          (center,start+1,start)]
            mesh = bpy.data.meshes.new(o.name+' / round radial roof')
            local = o.matrix_world.inverted()
            mesh.from_pydata([local@Vector(p) for p in vertices], [], faces)
            bm = bmesh.new(); bm.from_mesh(mesh)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            bad = sum(not e.is_manifold for e in bm.edges)
            degenerate = sum(f.calc_area() < 1e-8 for f in bm.faces)
            bm.to_mesh(mesh); bm.free()
            if bad or degenerate:
                raise ValueError(f'Invalid roof {o.name}: {bad}, {degenerate}')
            neutral = bpy.data.materials.get('Round two south gate neutral')
            if neutral is None:
                neutral = bpy.data.materials.new('Round two south gate neutral')
                neutral.diffuse_color = (.4,.4,.4,1)
            mesh.materials.append(neutral)
            uv = mesh.uv_layers.new(name='Source projection')
            for loop in mesh.loops:
                p = o.matrix_world@mesh.vertices[loop.vertex_index].co
                uv.data[loop.index].uv = (p.x/1920, 1-(-p.y*sine-p.z*cosine)/2752)
            o.data = mesh
            o['round2_south_gate'] = TAG
            o['roof_profile_power'] = power
            o['roof_radial_segments'] = count
        reports.append({'tower':name,'radial_segments':count,'profile_power':power,
                        'radius':radius,'base':base,'top':top,'closed':True})
    for low, high, cx, cy, lower, upper in (
            (5,6,754.3654,-4256.46,37,49.5),
            (3,4,942.7955,-4176.46,39,47.5)):
        round_drum(active[f'building-{low:03d}'],cx,cy,[(.0006,lower),(163,lower)])
        round_drum(active[f'building-{high:03d}'],cx,cy,
                   [(163,lower),(169,upper-2),(173,upper),(258,upper),
                    (260,upper+1),(263.0006,upper+1)])
    bpy.context.view_layer.update()
    return {'tag':TAG,'towers':reports}
