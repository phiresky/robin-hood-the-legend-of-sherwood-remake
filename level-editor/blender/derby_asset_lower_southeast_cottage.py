"""Close the southeast cottage shells and give its gabled thatch actual depth.

The source depicts a gable, not a hipped roof. Its ridge and footprint remain
measured anchors. Thatch thickness, overhang and doorway recess depths are
small authored estimates from the visible eaves and dark porch opening.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

IDS = (11, 59, 60, 61)
TAG = "southeast_cottage_refinement"
RECIPE = "southeast-closed-gable-thatch-porch-v1"
FALLBACK = "reprojection_fallback_material"


def _screen(v):
    return Vector((v.x, -v.y * math.sin(math.radians(35)) - v.z * math.cos(math.radians(35)), 1))


def _mesh(source, vertices, faces, mappings, name):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    for material in source.data.materials:
        mesh.materials.append(material)
    transforms = {}
    for index in set(mappings):
        face = source.data.polygons[index]
        matrix = Matrix([_screen(world[i]) for i in face.vertices]).transposed()
        if abs(matrix.determinant()) < 1e-7:
            raise ValueError(f"Degenerate source projection: {source.name} face {index}")
        transforms[index] = matrix.inverted()
    for old_uv in source.data.uv_layers:
        uv = mesh.uv_layers.new(name=old_uv.name)
        roof_bounds = None
        if 'thatch' in name and not old_uv.name.startswith('Refreshed map projection'):
            roof_uv = [old_uv.data[li].uv for p in source.data.polygons[6:8] for li in p.loop_indices]
            roof_bounds = [(min(p[i] for p in roof_uv), max(p[i] for p in roof_uv)) for i in range(2)]
        for face, index in zip(mesh.polygons, mappings):
            old = source.data.polygons[index]
            old_values = [old_uv.data[li].uv.copy() for li in old.loop_indices]
            for li in face.loop_indices:
                weights = transforms[index] @ _screen(vertices[mesh.loops[li].vertex_index])
                value = sum((old_values[i] * weights[i] for i in range(3)), Vector((0, 0)))
                if roof_bounds:
                    # Extra eave thickness must extend the roof texels, not
                    # sample the black padding outside its old atlas island.
                    value = Vector(tuple(min(hi, max(lo, value[i])) for i, (lo, hi) in enumerate(roof_bounds)))
                uv.data[li].uv = value
    backup = mesh.attributes.new(FALLBACK, "INT", "FACE")
    old_backup = source.data.attributes.get(FALLBACK)
    for face, index in zip(mesh.polygons, mappings):
        material = old_backup.data[index].value if old_backup else source.data.polygons[index].material_index
        face.material_index = backup.data[face.index].value = material
    bm = bmesh.new()
    try:
        bm.from_mesh(mesh)
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.00001)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        defects = {"nonmanifold": sum(not e.is_manifold for e in bm.edges),
                   "degenerate": sum(f.calc_area() <= 1e-6 for f in bm.faces)}
        if any(defects.values()):
            raise ValueError(f"Invalid {name}: {defects}")
        bm.to_mesh(mesh)
    finally:
        bm.free()
    return mesh, defects


def _prism(source, top, bottom, mapping, name):
    faces = [(0, 1, 2, 3), (7, 6, 5, 4)]
    faces += [(i, (i+1)%4, (i+1)%4+4, i+4) for i in range(4)]
    return _mesh(source, top + bottom, faces, mapping, name)


def _roof(source, number, shared):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    top = [world[i].copy() for i in ((12, 13, 14, 15) if number == 60 else (14, 13, 12, 15))]
    top[1], top[2] = [p.copy() for p in shared]
    lower = [p - Vector((0, 0, 3)) for p in top]
    bottom = [Vector((p.x, p.y, 0)) for p in lower]
    sides = [4, 2, 2, 0] if number == 60 else [4, 0, 0, 2]
    walls = _prism(source, lower, bottom, [6, 0] + sides, "Southeast cottage / closed supporting walls")
    # Expand only the outside eave: both roof halves meet at the same ridge.
    for outer, inner in ((0, 1), (3, 2)):
        direction = top[outer] - top[inner]
        direction.z = 0
        top[outer] += direction.normalized() * 2
    slab_bottom = [p - Vector((0, 0, 3)) for p in top]
    roof = _prism(source, top, slab_bottom, [6, 6, 6, 6, 6, 6], "Southeast cottage / three-unit thatch eave")
    return [("closed wall shell", walls), ("thatch roof with eave thickness", roof)]


def _porch(source):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    # Left outside wall is the visible narrow porch front. Keep its sloped cap.
    front_a, front_b, back_a, back_b = [world[i].copy() for i in (19, 16, 18, 17)]
    def point(u, depth, z):
        p = front_a.lerp(front_b, u).lerp(back_a.lerp(back_b, u), depth)
        p.z = z
        return p
    a,b,c,d = (point(0,0,0), point(1,0,0), point(1,1,0), point(0,1,0))
    lo,hi,sill,lintel,depth=.23,.67,1.0,44.0,.18
    opening=[point(lo,0,sill),point(lo,0,lintel),point(hi,0,lintel),point(hi,0,sill)]
    inset=[point(lo,depth,sill),point(lo,depth,lintel),point(hi,depth,lintel),point(hi,depth,sill)]
    vertices=[a,b,c,d,front_a,front_b,back_b,back_a]+opening+inset
    # Concave outline runs around the doorway; a low sill closes its bottom.
    faces=[(0,8,9,10,11,1,5,4),(8,11,1,0),(8,12,13,9),
           (9,13,14,10),(10,14,15,11),(8,11,15,12),(12,15,14,13),
           (4,5,6,7),(0,4,7,3),(1,2,6,5),(3,7,6,2),(0,3,2,1)]
    # Bottom front ring is a single quad between ground and doorway sill.
    faces[1]=(0,1,11,8)
    return [("porch with recessed doorway", _mesh(source,vertices,faces,
            [2,2,2,2,2,2,2,8,4,0,6,0],"Southeast cottage / recessed porch doorway"))]


def _wedge(source):
    world=[source.matrix_world@v.co for v in source.data.vertices]
    apex=world[9].copy()
    base=[Vector((p.x,p.y,0)) for p in (world[9],world[10],world[8])]
    return [("closed end extension",_mesh(source,base+[apex],
             [(0,2,1),(0,1,3),(1,2,3),(2,0,3)],[4,1,4,2],
             "Southeast cottage / repaired triangular end extension"))]


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.objects if o.get(TAG)==RECIPE and not o.hide_render]
    if existing:
        counts={f'building-{n:03}':sum(o.get('source_node')==f'building-{n:03}' for o in existing) for n in IDS}
        if len(existing)!=6 or counts!={'building-011':1,'building-059':1,'building-060':2,'building-061':2}:
            raise ValueError('Incomplete southeast cottage refinement')
        return {'status':'existing','objects':[o.name for o in existing]}
    bpy.context.view_layer.update()
    originals={}
    for number in IDS:
        candidates=[o for o in working.objects if o.type=='MESH' and not o.hide_render
                    and o.get('source_node')==f'building-{number:03}']
        if len(candidates)!=1:raise ValueError(f'Expected one source for {number}')
        originals[number]=candidates[0]
    east,west=originals[60],originals[61]
    shared=[(east.matrix_world@east.data.vertices[a].co+west.matrix_world@west.data.vertices[b].co)/2
            for a,b in ((13,13),(14,12))]
    report=[]
    for number,source in originals.items():
        pieces=(_roof(source,number,shared) if number in (60,61)
                else _porch(source) if number==59 else _wedge(source))
        for label,(mesh,defects) in pieces:
            obj=bpy.data.objects.new(source.name+' / '+label,mesh)
            working.objects.link(obj)
            obj.parent=source.parent
            bpy.context.view_layer.update()
            obj.matrix_world=Matrix.Identity(4)
            for key,value in source.items():obj[key]=value
            obj[TAG]=RECIPE
            obj['refinement_recipe']=RECIPE
            obj['part_name']=source['part_name']
            report.append({'source_node':source['source_node'],'piece':label,
                           'faces':len(mesh.polygons),'validation':defects})
        source.hide_render=source.hide_viewport=True
    bpy.context.view_layer.update()
    return {'status':'created','objects':report,
            'audit':{'011':'Nearly degenerate open triangular wedge closed; silhouette retained',
                     '059':'Porch seams closed; painted opening recessed',
                     '060':'Gable retained; shared ridge, closed walls and thatch eave thickness',
                     '061':'Gable retained; shared ridge, closed walls and thatch eave thickness'},
            'uncertainty':'Door depth and three-unit thatch thickness are authored estimates; no source evidence supports a hipped roof',
            'remaining':'Fine thatch strands and individual timber joints remain texture detail; concealed walls retain fallback atlas'}


if __name__=='__main__':result=refine()
