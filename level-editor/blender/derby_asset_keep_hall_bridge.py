"""Open the painted bridge arch and separate parapets from solid collision slabs."""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

TAG='keep-hall-open-arch-v1'


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.all_objects if o.get('bridge_arch_refinement')==TAG]
    if existing:
        if len(existing)!=4: raise ValueError('Incomplete bridge refinement')
        return {'status':'existing','parts':len(existing)}
    bpy.context.view_layer.update()
    sources={n:next(o for o in working.all_objects if o.get('source_node')==f'building-{n}' and not o.hide_render) for n in (120,122,126,129)}
    world={n:[o.matrix_world@v.co for v in o.data.vertices] for n,o in sources.items()}
    rings={120:list(range(20,26)),122:list(range(16,20)),126:list(range(24,30)),129:list(range(16,20))}
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    def projected(v):return Vector((v.x,-v.y*sine-v.z*cosine,1))
    generated={}
    for n,source in sources.items():
        top=[world[n][i].copy() for i in rings[n]]; count=len(top)
        base=152.4 if n==129 else 0
        verts=top+[Vector((v.x,v.y,base)) for v in top]
        faces=[tuple(range(count)),tuple(reversed(range(count,count*2)))]+[(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
        mesh=bpy.data.meshes.new(source.name+' closed bridge masonry');mesh.from_pydata(verts,[],faces);mesh.update()
        mappings=[]
        for face in source.data.polygons:
            matrix=Matrix([projected(world[n][i]) for i in face.vertices]).transposed()
            if abs(matrix.determinant())>1e-8:mappings.append((face,matrix.inverted(),[source.data.uv_layers.active.data[i].uv.copy() for i in face.loop_indices]))
        uv=mesh.uv_layers.new(name='UVMap')
        for polygon in mesh.polygons:
            center=sum((verts[i] for i in polygon.vertices),Vector())/len(polygon.vertices)
            def score(entry):
                face=entry[0];normal=(source.matrix_world.to_3x3()@face.normal).normalized();fc=sum((world[n][i] for i in face.vertices),Vector())/len(face.vertices)
                return abs(normal.dot(polygon.normal))-abs((center-fc).dot(normal))*.01
            face,inverse,triuv=max(mappings,key=score)
            for loop in polygon.loop_indices:
                weights=inverse@projected(verts[mesh.loops[loop].vertex_index]);uv.data[loop].uv=sum((triuv[i]*weights[i] for i in range(3)),Vector((0,0)))
        for material in source.data.materials:mesh.materials.append(material)
        bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
        obj=bpy.data.objects.new(source.name+' / Open arch masonry',mesh);working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
        for key in source.keys():obj[key]=source[key]
        generated[n]=obj
    # The painted arch spans most of the bridge, leaving substantial abutments.
    start,end=world[126][24].copy(),world[126][29].copy();start.z=end.z=0
    axis=end-start;normal=Vector((-axis.y,axis.x,0)).normalized()
    profile=[(.07,-20),(.86,-20),(.86,12)]
    for i in range(1,33):
        angle=math.pi*i/32
        profile.append((.465+.395*math.cos(angle),12+91*math.sin(angle)))
    points=[]
    for depth in (-100,100):
        points.extend(start+axis*u+normal*depth+Vector((0,0,z)) for u,z in profile)
    count=len(profile);faces=[tuple(reversed(range(count))),tuple(range(count,count*2))]+[(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
    mesh=bpy.data.meshes.new('Bridge arch cutter');mesh.from_pydata(points,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    cutter=bpy.data.objects.new('Bridge arch cutter',mesh);working.objects.link(cutter)
    bpy.context.view_layer.update()
    try:
        for n in (120,126):
            obj=generated[n];modifier=obj.modifiers.new('Masonry arch opening','BOOLEAN');modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
            bpy.context.view_layer.objects.active=obj;bpy.ops.object.modifier_apply(modifier=modifier.name)
    finally:
        bpy.data.objects.remove(cutter,do_unlink=True);bpy.data.meshes.remove(mesh)
    # Two broad lowered stretches are visible between the rear wall's raised
    # masonry sections. Keep the lower coping continuous above the deck.
    back_start,back_end=world[129][17].copy(),world[129][16].copy()
    back_start.z=back_end.z=0
    back_axis=back_end-back_start
    back_normal=Vector((-back_axis.y,back_axis.x,0)).normalized()
    for low,high in ((.17,.27),(.38,.64)):
        points=[back_start+back_axis*u+back_normal*depth+Vector((0,0,z))
                for z in (173,205) for depth in (-20,20) for u in (low,high)]
        mesh=bpy.data.meshes.new('Rear coping notch cutter')
        mesh.from_pydata(points,[],[(0,2,3,1),(4,5,7,6),(0,1,5,4),(2,6,7,3),(0,4,6,2),(1,3,7,5)]);mesh.update()
        cutter=bpy.data.objects.new('Rear coping notch cutter',mesh);working.objects.link(cutter)
        bpy.context.view_layer.update()
        try:
            obj=generated[129];modifier=obj.modifiers.new('Lowered rear coping','BOOLEAN');modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
            bpy.context.view_layer.objects.active=obj;bpy.ops.object.modifier_apply(modifier=modifier.name)
        finally:
            bpy.data.objects.remove(cutter,do_unlink=True);bpy.data.meshes.remove(mesh)
    reports=[]
    for n,obj in generated.items():
        bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bmesh.ops.triangulate(bm,faces=list(bm.faces))
        bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=2,use_grid_fill=True);bmesh.ops.triangulate(bm,faces=list(bm.faces))
        defects={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),'degenerate_faces':sum(f.calc_area()<1e-7 for f in bm.faces)}
        if any(defects.values()):bm.free();raise ValueError(f'{n}: {defects}')
        bm.to_mesh(obj.data);bm.free();obj['bridge_arch_refinement']=TAG
        sources[n].hide_render=sources[n].hide_viewport=True
        reports.append({'source':obj['source_node'],'faces':len(obj.data.polygons),'validation':defects})
    return {'status':'refined','parts':reports,'arch_crown_scene_z':103,'rear_coping_notches':2,'limitations':['Hidden arch masonry uses existing fallback texture','West landing short tread count remains uncertain']}
