"""Closed cottage shells, rounded thatch and the painted torn-roof cavity."""
import math

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = 'southwest_cottage_refinement'
FALLBACK = 'reprojection_fallback_material'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))


def _project(p):
    return Vector((p.x, -p.y * SIN - p.z * COS, 1))


def _mesh(source, ridge):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    east = source['source_node'] == 'building-058'
    eave = [world[i].copy() for i in ((19, 16) if east else (16, 19))]
    # The thatch extends below the original eastern collision slope. The source
    # shows a soft shoulder and lower eave; no hidden-side doorway is invented.
    if east:
        for p in eave:
            p.z -= 12
    rows = (0, .25, .5, .75, 1)
    vertices = []
    for t in rows:
        for a,b in zip(ridge,eave):
            p = a.lerp(b,t)
            p.z += 5 * 4*t*(1-t)
            vertices.append(p)
    vertices += [Vector((p.x,p.y,0)) for p in (ridge[0],ridge[1],eave[1],eave[0])]
    faces = [(2*i,2*i+1,2*i+3,2*i+2) for i in range(4)]
    faces.extend(((10,11,1,0), (11,12,9,7,5,3,1),
                  (12,13,8,9), (13,10,0,2,4,6,8), (13,12,11,10)))
    mesh = bpy.data.meshes.new(source.name+' / closed thatch shell')
    mesh.from_pydata(vertices,[],faces)
    mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bm.to_mesh(mesh);bm.free()
    return mesh


def _uv(source, mesh):
    world=[source.matrix_world@v.co for v in source.data.vertices]
    normal=source.matrix_world.to_3x3().inverted().transposed()
    donors=[]
    roof_points=[world[i] for i in source.data.polygons[8].vertices]
    roof_normal=(roof_points[1]-roof_points[0]).cross(roof_points[2]-roof_points[0])
    source_uv=source.data.uv_layers.active
    saved=source.data.attributes.get(FALLBACK)
    for p in source.data.polygons:
        inverse=Matrix([_project(world[i]) for i in p.vertices]).transposed()
        if abs(inverse.determinant())<1e-8:
            continue
        donors.append((p.index,(normal@p.normal).normalized(),inverse.inverted(),
                       [source_uv.data[i].uv.copy() for i in p.loop_indices],
                       saved.data[p.index].value if saved else p.material_index))
    mesh.materials.clear()
    for mat in source.data.materials:
        mesh.materials.append(mat)
    for layer in list(mesh.uv_layers):
        mesh.uv_layers.remove(layer)
    uv=mesh.uv_layers.new(name='Southwest cottage fallback atlas')
    old=mesh.attributes.get(FALLBACK)
    if old:
        mesh.attributes.remove(old)
    saved=mesh.attributes.new(FALLBACK,'INT','FACE')
    for p in mesh.polygons:
        donor=max(donors,key=lambda d:d[1].dot(p.normal))
        cavity=p.center.z>90 and 520<_project(p.center).x<558 and 2144<_project(p.center).y<2183
        if cavity and p.normal.z<.15:
            donor=next(d for d in donors if d[0]==8)
        _,_,inverse,coords,material=donor
        for li in p.loop_indices:
            vertex=mesh.vertices[mesh.loops[li].vertex_index].co
            sample=vertex.copy()
            if donor[0] in (8,9) and not cavity:
                # A rounded shoulder can extend outside the old cropped atlas.
                # Parameterize its fallback on the original roof plane; visible
                # faces receive exact world projection in the following pass.
                sample.z=roof_points[0].z-(roof_normal.x*(sample.x-roof_points[0].x)
                                          +roof_normal.y*(sample.y-roof_points[0].y))/roof_normal.z
            point=_project(sample)
            if cavity and p.normal.z<.15:
                # The newly exposed thatch cut has no photographed side view;
                # retain a dark donor inside the observed opening.
                point=Vector((542+(point.x-542)*.08,2163+(point.y-2163)*.08,1))
            weights=inverse@point
            uv.data[li].uv=sum((coords[i]*weights[i] for i in range(3)),Vector((0,0)))
        p.material_index=material
        saved.data[p.index].value=material


def _opening(obj):
    mesh=obj.data
    mesh.calc_loop_triangles()
    # Screen-space torn perimeter follows the dark hole, excluding the visible
    # diagonal wood rafter. The pocket exposes real depth below the thatch.
    outline=((532,2152),(548,2153),(552,2161),(546,2169),(536,2171),(529,2165))
    points=[]
    for x,y in outline:
        point=None
        for tri in mesh.loop_triangles:
            if tri.normal.z<=.1:
                continue
            verts=[mesh.vertices[i].co for i in tri.vertices]
            matrix=Matrix([_project(v) for v in verts]).transposed()
            if abs(matrix.determinant())<1e-8:
                continue
            w=matrix.inverted()@Vector((x,y,1))
            if min(w)>-.001:
                point=sum((verts[i]*w[i] for i in range(3)),Vector());break
        if point is None:
            raise ValueError(f'Torn roof edge outside top surface: {x},{y}')
        points.append(point)
    vertices=[p+Vector((0,0,dz)) for dz in (2,-13) for p in points]
    n=len(points)
    faces=[tuple(range(n)),tuple(range(n,2*n))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    cut=bpy.data.meshes.new('Southwest cottage roof opening cutter')
    cut.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(cut)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bmesh.ops.triangulate(bm,faces=list(bm.faces))
    bm.to_mesh(cut);bm.free()
    cutter=bpy.data.objects.new(cut.name,cut)
    bpy.context.scene.collection.objects.link(cutter)
    modifier=obj.modifiers.new('Painted torn-roof opening','BOOLEAN')
    modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
    bpy.context.view_layer.objects.active=obj
    try:
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    finally:
        bpy.data.objects.remove(cutter,do_unlink=True)
        if cut.users==0:bpy.data.meshes.remove(cut)
    return {'depth':13,'source_perimeter':outline}


def _concealed_wall_fallback(mesh, donor):
    """Replace foreground castle-brick contamination with this cottage's plaster.

    The small unobstructed plaster donor deliberately adds no speculative doors
    or windows to unseen walls. Visible faces are refreshed in the normal pass.
    """
    old=donor.data.polygons[0]
    corners=[donor.matrix_world@donor.data.vertices[i].co for i in old.vertices]
    inverse=Matrix([_project(p) for p in corners]).transposed().inverted()
    coords=[donor.data.uv_layers.active.data[i].uv.copy() for i in old.loop_indices]
    old_saved=donor.data.attributes.get(FALLBACK)
    material=donor.data.materials[old_saved.data[old.index].value if old_saved else old.material_index]
    slot=next((i for i,m in enumerate(mesh.materials) if m==material),None)
    if slot is None:
        slot=len(mesh.materials);mesh.materials.append(material)
    uv=mesh.uv_layers.active
    backup=mesh.attributes[FALLBACK]
    changed=0
    for p in mesh.polygons:
        if abs(p.normal.z)>.05:
            continue
        center=_project(p.center)
        for li in p.loop_indices:
            projected=_project(mesh.vertices[mesh.loops[li].vertex_index].co)
            dx=max(-3,min(3,(projected.x-center.x)*.07))
            dy=max(-3,min(3,(projected.y-center.y)*.07))
            weights=inverse@Vector((588+dx,2228+dy,1))
            uv.data[li].uv=sum((coords[i]*weights[i] for i in range(3)),Vector((0,0)))
        p.material_index=slot;backup.data[p.index].value=slot;changed+=1
    return changed


def refine():
    collection=bpy.data.collections['Derby Working']
    previous=[o for o in collection.objects if o.get(TAG)]
    if previous:
        if {o.get('source_node') for o in previous}!={'building-057','building-058'}:
            raise ValueError('Incomplete southwest cottage refinement')
        return {'status':'existing','objects':[o.name for o in previous]}
    bpy.context.view_layer.update()
    sources=[next(o for o in collection.objects if o.get('source_node')==f'building-{i:03}' and not o.hide_render) for i in (57,58)]
    west,east=sources
    ridge=[(west.matrix_world@west.data.vertices[a].co+east.matrix_world@east.data.vertices[b].co)/2
           for a,b in ((17,18),(18,17))]
    report=[]
    for source in sources:
        mesh=_mesh(source,ridge)
        replacement=bpy.data.objects.new(source.name+' / reviewed thatch',mesh)
        collection.objects.link(replacement)
        replacement.parent=source.parent;replacement.matrix_world=Matrix.Identity(4)
        for key in source.keys():replacement[key]=source[key]
        replacement[TAG]=1
        bpy.context.view_layer.update()
        opening=_opening(replacement) if source==east else None
        mesh=replacement.data
        _uv(source,mesh)
        concealed=_concealed_wall_fallback(mesh,east) if source==west else 0
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bmesh.ops.triangulate(bm,faces=list(bm.faces))
        defects={'nonmanifold':sum(not e.is_manifold for e in bm.edges),
                 'degenerate':sum(f.calc_area()<1e-6 for f in bm.faces)}
        if any(defects.values()):
            bm.free();raise ValueError(f'Invalid southwest cottage geometry: {defects}')
        bm.to_mesh(mesh);bm.free()
        source.hide_render=True;source.hide_set(True)
        report.append({'source_node':source['source_node'],'opening':opening,
                       'concealed_plaster_fallback_faces':concealed,'validation':defects})
    bpy.context.view_layer.update()
    return {'status':'created','objects':report,
            'remaining':'Concealed west walls use a small observed cottage plaster donor; rear details and complete interior are not inferred.'}
