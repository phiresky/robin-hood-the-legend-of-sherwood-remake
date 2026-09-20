"""Close gate support seams and model the source-supported roof profiles."""
import math
import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-bailey-gate'
TAG = 'east-bailey-gate-round2-v1'


def _hull(points):
    points = sorted(set((round(p[0], 5), round(p[1], 5)) for p in points))
    def cross(a, b, c):
        return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    sides = []
    for seq in (points, list(reversed(points))):
        side = []
        for p in seq:
            while len(side)>1 and cross(side[-2], side[-1], p)<=.1:
                side.pop()
            side.append(p)
        sides.append(side[:-1])
    return sides[0]+sides[1]


def _points(obj):
    return [obj.matrix_world @ v.co for v in obj.data.vertices]


def _replace(obj, points, faces):
    mesh = bpy.data.meshes.new(obj.name+' / coherent shell')
    inv = obj.matrix_world.inverted()
    mesh.from_pydata([inv@Vector(p) for p in points], [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad = sum(not e.is_manifold for e in bm.edges)
    degenerate = sum(f.calc_area()<1e-7 for f in bm.faces)
    if bad or degenerate:
        raise ValueError((obj.name, bad, degenerate))
    bm.to_mesh(mesh)
    bm.free()
    # Source projection is rebuilt after geometry; no generated appearance is inherited.
    mat = bpy.data.materials.get('East Bailey round2 source pending')
    if mat is None:
        mat = bpy.data.materials.new('East Bailey round2 source pending')
        mat.diffuse_color = (.35,.35,.35,1)
    mesh.materials.append(mat)
    mesh.uv_layers.new(name='UVMap')
    obj.data = mesh
    obj[TAG] = True


def _prism(obj, xy, top):
    n = len(xy)
    points = [(x,y,z) for z in (0,top) for x,y in xy]
    faces = [tuple(reversed(range(n))), tuple(range(n,2*n))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    _replace(obj, points, faces)


def _clip(poly, point, normal):
    out = []
    for a,b in zip(poly, poly[1:]+poly[:1]):
        da = (Vector(a)-point).dot(normal)
        db = (Vector(b)-point).dot(normal)
        if da >= 0:
            out.append(a)
        if (da>=0)!=(db>=0):
            out.append(tuple(Vector(a).lerp(Vector(b),da/(da-db))))
    return out


def _support(obj, footprint, underside):
    original=_hull(_points(obj))
    top=max(p.z for p in _points(obj))
    _prism(obj,original,top)
    support=bpy.data.objects.new('Temporary gate support extension',obj.data.copy())
    bpy.context.scene.collection.objects.link(support)
    support.matrix_world=obj.matrix_world.copy()
    _prism(support,footprint,underside)
    try:
        mod=obj.modifiers.new('Continuous support beneath parapet','BOOLEAN')
        mod.operation='UNION'
        mod.solver='EXACT'
        mod.object=support
        bpy.context.view_layer.objects.active=obj
        bpy.ops.object.modifier_apply(modifier=mod.name)
    finally:
        bpy.data.objects.remove(support,do_unlink=True)
    bm=bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.001)
    bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=.0001)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges):
        raise ValueError(('Nonmanifold gate support union',obj.name,
                          sum(not e.is_manifold for e in bm.edges)))
    bm.to_mesh(obj.data)
    bm.free()


def _section_shell(obj, center, corners, sections):
    n=len(corners)
    vertices=[]
    for z,scale in sections:
        vertices.extend([(center.x+(p.x-center.x)*scale,
                          center.y+(p.y-center.y)*scale,z) for p in corners])
    faces=[tuple(reversed(range(n)))]
    for k in range(len(sections)-1):
        for i in range(n):
            j=(i+1)%n
            faces.append((k*n+i,k*n+j,(k+1)*n+j,(k+1)*n+i))
    faces.append(tuple(range((len(sections)-1)*n,len(sections)*n)))
    _replace(obj, vertices, faces)


def refine():
    owned=[o for o in bpy.data.collections['Derby Working'].all_objects
           if o.type=='MESH' and o.get('asset_group')==ASSET and not o.hide_render]
    existing=[o for o in owned if o.get(TAG)]
    if existing:
        if len(existing)!=5 or {o.get('source_node') for o in existing}!={
                'building-079','building-080','building-084','building-085'}:
            raise ValueError('Incomplete East Bailey gate second pass')
        return {'status':'existing','asset':ASSET}
    def part(n,round_turret=False):
        found=[o for o in owned if o.get('source_node')==f'building-{n:03}'
               and ('Round corner' in o.name)==round_turret]
        if len(found)!=1:
            raise ValueError((n,[o.name for o in found]))
        return found[0]
    # The upper wall's front footprint projects beyond its supporting piers.
    # Extend the support footprints, leaving each passage's jamb plane intact.
    left=part(85); right=part(84)
    left_wall=_hull(_points(part(95)))
    right_wall=_hull(_points(part(96)))
    jamb=Vector((1072.706,-3019.666))
    span=Vector((1072.706-1017.992,-3019.666+2989.184)).normalized()
    right_wall=_clip(right_wall,jamb,span)
    # End exactly at the upper shell's underside: coplanar overlapping facade
    # faces would fight during textured rendering and confuse source ownership.
    _support(left,_hull(_points(left)+[Vector((x,y,0)) for x,y in left_wall]),
           min(p.z for p in _points(part(95))))
    _support(right,_hull(_points(right)+[Vector((x,y,0)) for x,y in right_wall]),
           min(p.z for p in _points(part(96))))
    # Reuse the shared original eave corners and roof axis. Both closed halves
    # receive identical sections, so their internal diagonal cannot split apart.
    body_points=_points(part(76))
    corners=[Vector((x,y,259)) for x,y in _hull([p for p in body_points if p.z>258.99])]
    if len(corners)!=4:
        raise ValueError(('Expected four tower eave corners',corners))
    apex=max(_points(part(79)),key=lambda p:p.z)
    # A small common axis correction and a single radial exponent fit the
    # reviewed silhouette while retaining coherent four-sided sections.
    apex.x-=1.25
    apex.y+=3.5
    sections=[(259,1),(261,1.035),(269,.92),(283,.78),(300,.63),(319,.47),
              (339,.33),(357,.215),(363,.18),(364,.18),(386,.037),
              (389,.026),(390,.07),(397,.021),(415,.008),(418.08644,.003)]
    sections=[(z,scale**.92 if 269<=z<=363 else scale) for z,scale in sections]
    # These two triangular sections tile the rectangular roof without an overlap.
    _section_shell(part(79),apex,[corners[0],corners[1],corners[3]],sections)
    _section_shell(part(80),apex,[corners[1],corners[2],corners[3]],sections)
    small=part(80,True)
    points=_points(small)
    center=max(points,key=lambda p:p.z)
    ring=[Vector((center.x+21*math.cos(i*2*math.pi/48),
                  center.y+21*math.sin(i*2*math.pi/48),259)) for i in range(48)]
    _section_shell(small,center,ring,[(259,1),(261,1),(268,.86),(280,.68),
                   (294,.48),(308,.30),(317,.205),(319,.205),(331,.052),
                   (333,.045),(334,.12),(338,.04),(347,.012)])
    bpy.context.view_layer.update()
    return {'asset':ASSET,'status':'changed','parts':[79,80,84,85],
            'support_plane':'Continuous full-height pier footprint under parapet',
            'roof_sections':len(sections),'changed_meshes':5}
