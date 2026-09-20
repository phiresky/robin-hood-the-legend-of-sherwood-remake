"""Restore the southwest turret roof and close the curtain's support assemblies.

The reviewed roof silhouette supplies the profile, including its narrow finial.
The concealed continuation is rotational symmetry, not recovered decoration.
Existing source parts and object transforms are retained for editor selection.
"""
import math
import json
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-lower-west-curtain'
TAG = 'lower-west-curtain-curved-roof-v1'
# Radius/height knots fitted against the reviewed turret silhouette. The roof
# overlaps the shaft by 1.25 units at its underside instead of floating above it.
PROFILE = [(245., 36.), (260., 30.24), (275., 21.10), (290., 16.58),
           (310., 10.31), (330., 3.66), (345., 2.42), (360., .86),
           (376.5, .22), (378., 0.)]
SECTORS = {'building-028': (-112, -66), 'building-029': (-158, -112),
           'building-030': (92, 202), 'building-031': (-27, 92),
           'building-032': (-66, -27)}


def _radius(z):
    """Monotone cubic interpolation, avoiding overshoot near the narrow tip."""
    slopes = [(b[1]-a[1])/(b[0]-a[0]) for a,b in zip(PROFILE, PROFILE[1:])]
    tangents = [slopes[0]] + [2*a*b/(a+b) if a*b>0 else 0
                             for a,b in zip(slopes,slopes[1:])] + [slopes[-1]]
    for i,(a,b) in enumerate(zip(PROFILE,PROFILE[1:])):
        if a[0] <= z <= b[0]:
            h=b[0]-a[0]; t=(z-a[0])/h
            return ((2*t**3-3*t**2+1)*a[1] + (t**3-2*t**2+t)*h*tangents[i]
                    + (-2*t**3+3*t**2)*b[1] + (t**3-t**2)*h*tangents[i+1])
    raise ValueError('Roof profile height outside fitted range')


def _replace_roof(obj, angles):
    a,b=map(math.radians,angles)
    steps=math.ceil((angles[1]-angles[0])/4)
    angles=[a+(b-a)*i/steps for i in range(steps+1)]
    rings=[(243.,36.)]+[(z,_radius(z)) for z in
        sorted(set([p[0] for p in PROFILE[:-1]] + [245+i*4 for i in range(33)]))]
    world=[(371.,-3894.,243.),(371.,-3894.,378.)]
    for z,r in rings:
        world.extend((371+r*math.cos(t),-3894+r*math.sin(t),z) for t in angles)
    n=len(angles)
    faces=[]
    for j in range(len(rings)-1):
        for i in range(n-1):
            k=2+j*n+i
            faces.append((k,k+1,k+1+n,k+n))
    for i in range(n-1):
        faces.append((0,2+i+1,2+i))
        k=2+(len(rings)-1)*n+i
        faces.append((k,k+1,1))
    # Closed sector cuts meet neighboring cuts exactly without exposed gaps.
    faces.append(tuple([0]+[2+j*n for j in range(len(rings))]+[1]))
    faces.append(tuple([0,1]+[2+j*n+n-1 for j in reversed(range(len(rings)))]))
    inv=obj.matrix_world.inverted()
    mesh=bpy.data.meshes.new(obj.name+' / curved closed shell')
    mesh.from_pydata([inv@Vector(p) for p in world],[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bad=sum(not e.is_manifold for e in bm.edges)
    deg=sum(f.calc_area()<1e-8 for f in bm.faces)
    volume=bm.calc_volume(signed=True)
    if bad or deg or volume<=0:
        bm.free()
        raise ValueError(f'Invalid roof {obj.name}: {bad} edges, {deg} faces, volume {volume}')
    bm.to_mesh(mesh);bm.free()
    for mat in obj.data.materials:mesh.materials.append(mat)
    uv=mesh.uv_layers.new(name=obj.data.uv_layers[0].name)
    for loop in mesh.loops:
        x,y,z=world[loop.vertex_index]
        uv.data[loop.index].uv=(x/1920,1-(-y*math.sin(math.radians(35))-z*math.cos(math.radians(35)))/2752)
    mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    obj.data=mesh
    obj['round2_lower_west_curtain']=TAG
    return {'source_node':obj['source_node'],'vertices':len(mesh.vertices),
            'faces':len(mesh.polygons),'nonmanifold_edges':bad,
            'degenerate_faces':deg,'positive_volume':volume}


def refine():
    objects=[o for o in bpy.data.collections['Derby Working'].objects
             if o.type=='MESH' and not o.hide_render and o.get('asset_group')==ASSET]
    roofs={node:[o for o in objects if o.get('source_node')==node] for node in SECTORS}
    if any(len(v)!=1 for v in roofs.values()):
        raise ValueError('Expected exactly one active mesh per roof source part')
    reports=[_replace_roof(roofs[node][0],angles) for node,angles in SECTORS.items()]
    return {'asset_id':ASSET,'tag':TAG,'changed':reports,
            'unchanged_parts':[o.get('source_node') for o in objects if o.get('source_node') not in SECTORS],
            'source_evidence':'Reviewed roof silhouette; map box (335,1925,73,359), upper component only',
            'inferred':'Concealed rear shell rotational continuation; no generated surface detail'}


def refine_supports(level_path):
    """Rebuild three cracked panel assemblies from their authored closed plans.

    This does not fill arbitrary boundary loops. The authored footprints define
    the wall return, stair support and continuous walkway explicitly, including
    all their concave turns. Existing stair treads and accepted roof are retained.
    Run source reprojection after this geometry-only operation.
    """
    from mathutils.geometry import tessellate_polygon

    level = json.loads(Path(level_path).read_text())
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render and o.get('asset_group') == ASSET]
    reports = []
    for number in (37, 38, 45):
        node = f'building-{number:03d}'
        candidates = [o for o in objects if o.get('source_node') == node
                      and not o.get('lower_west_stair_refinement')]
        if len(candidates) != 1:
            raise ValueError(f'Expected one baseline support for {node}')
        obj = candidates[0]
        points = level['sight_obstacles'][number]['points']
        n = len(points)
        if n < 3:
            raise ValueError(f'Incomplete authored footprint for {node}')
        # Triangulate the plan, then use the same triangles at each vertex's
        # authored height. Stair height varies; the wall and walk are level.
        plan = [Vector((p['x'], -p['y']/sine, 0)) for p in points]
        lookup = {tuple(v): i for i,v in enumerate(plan)}
        triangles = [[v if isinstance(v, int) else lookup[tuple(v)] for v in tri]
                     for tri in tessellate_polygon([plan])]
        if len(triangles) != n-2:
            raise ValueError(f'Incomplete plan triangulation for {node}')
        world = [Vector((p['x'], -p['y']/sine, p[height]/cosine))
                 for height in ('z_bottom', 'z_top') for p in points]
        faces = [tuple(reversed(t)) for t in triangles]
        faces += [tuple(i+n for i in t) for t in triangles]
        faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
        mesh = bpy.data.meshes.new(obj.name+' / continuous authored support')
        inverse = obj.matrix_world.inverted()
        mesh.from_pydata([inverse@v for v in world], [], faces)
        bm=bmesh.new(); bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bad=sum(not e.is_manifold for e in bm.edges)
        degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
        volume=bm.calc_volume(signed=True)
        if bad or degenerate or volume <= 0:
            bm.free()
            raise ValueError(f'Invalid support {node}: {bad}, {degenerate}, {volume}')
        bm.to_mesh(mesh); bm.free()
        for mat in obj.data.materials:
            mesh.materials.append(mat)
        uv=mesh.uv_layers.new(name=obj.data.uv_layers[0].name)
        for loop in mesh.loops:
            p=world[loop.vertex_index]
            uv.data[loop.index].uv=(p.x/1920,1-(-p.y*sine-p.z*cosine)/2752)
        mesh.attributes.new('reprojection_fallback_material','INT','FACE')
        before_vertices=len(obj.data.vertices)
        obj.data=mesh
        obj['round2_lower_west_support']='authored-closed-support-v1'
        reports.append({'source_node':node,'before_vertices':before_vertices,
                        'vertices':len(mesh.vertices),'faces':len(mesh.polygons),
                        'authored_plan_points':n,'nonmanifold_edges':bad,
                        'degenerate_faces':degenerate,'positive_volume':volume})
    return {'asset_id':ASSET,'changed':reports,'preserved_roof_nodes':list(SECTORS),
            'evidence':'Closed authored wall/stair/walk footprints; source roof and stair treads retained'}
