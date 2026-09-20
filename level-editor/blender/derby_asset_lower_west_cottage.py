"""West cottage: source-fitted thick thatch, eaves and porch timbers.

Migrates the closed collision-prism pass, keeping all earlier meshes hidden.
Run layered reprojection afterwards. Grazing sidewalls stay unknown for texture
completion; no concealed openings are inferred from stretched source pixels.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

TAG='west-cottage-thatch-eaves-v4-level-ridge'
KEY='west_cottage_shell'
SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
RIDGE_HEIGHT,EAVE_HEIGHT=121.53,87.0

# Rear thatch outline measured in the covered artwork's pixel coordinates.
# The collision-derived rear edge includes a strip of the terrain behind it.
REAR_OUTLINE={
    55:((621,1847),(626,1854),(632,1860),(638,1864),(644,1868),
        (650,1872),(656,1876),(662,1880),(666,1885)),
    56:((621,1847),(616,1856),(611,1861),(606,1865),(601,1868),
        (596,1871),(591,1873),(586,1875),(582,1877)),
}


def _rear_shift(point,target):
    screen=_screen(point)
    return Vector((target[0]-screen.x,-(target[1]-screen.y)/SIN,0))


def _level_preserving_projection(point,height):
    point=point.copy()
    point.y-=(height-point.z)*COS/SIN
    point.z=height
    return point


def _screen(p):
    return Vector((p.x,-p.y*SIN-p.z*COS,1))


def _world(source):
    hidden=source.hide_viewport
    try:
        source.hide_viewport=False
        bpy.context.view_layer.update()
        matrix=source.matrix_world.copy()
        return matrix,[matrix@v.co for v in source.data.vertices]
    finally:
        source.hide_viewport=hidden


def _shell(name,points,rows,columns,thickness):
    vertices=points+[p-Vector((0,0,thickness)) for p in points]
    n=len(points);faces=[]
    for row in range(rows-1):
        for col in range(columns-1):
            a=row*columns+col;face=(a,a+1,a+1+columns,a+columns)
            faces.extend((face,tuple(i+n for i in reversed(face))))
    boundary=list(range(columns))+[r*columns+columns-1 for r in range(1,rows)]
    boundary+=list(range((rows-1)*columns+columns-2,(rows-1)*columns-1,-1))
    boundary+=[r*columns for r in range(rows-2,0,-1)]
    for i,a in enumerate(boundary):
        b=boundary[(i+1)%len(boundary)];faces.append((a,b,b+n,a+n))
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    return mesh


def _solid(name,top):
    n=len(top);vertices=top+[Vector((p.x,p.y,0)) for p in top]
    faces=[tuple(range(n)),tuple(reversed(range(n,2*n)))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    return mesh


def _roof_shell(name,points,top_faces,thickness):
    n=len(points);vertices=points+[p-Vector((0,0,thickness)) for p in points]
    faces=list(top_faces)+[tuple(i+n for i in reversed(face)) for face in top_faces]
    edges={}
    for face in top_faces:
        for a,b in zip(face,face[1:]+face[:1]):
            key=tuple(sorted((a,b)))
            if key in edges:edges[key]=None
            else:edges[key]=(a,b)
    for edge in edges.values():
        if edge is not None:
            a,b=edge;faces.append((a,b,b+n,a+n))
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    return mesh


def _finish(obj,source,matrix,world,subdivide=False):
    mesh=obj.data
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    normal=matrix.to_3x3().inverted().transposed();donors=[]
    old_uv=source.data.uv_layers.active
    saved=source.data.attributes.get('reprojection_fallback_material')
    for p in source.data.polygons:
        if len(p.vertices)!=3:continue
        inverse=Matrix([_screen(world[i]) for i in p.vertices]).transposed()
        if abs(inverse.determinant())<1e-8:continue
        donors.append(((normal@p.normal).normalized(),inverse.inverted(),
                       [old_uv.data[i].uv.copy() for i in p.loop_indices],
                       saved.data[p.index].value if saved else p.material_index))
    for layer in list(mesh.uv_layers):mesh.uv_layers.remove(layer)
    old=mesh.attributes.get('reprojection_fallback_material')
    if old:mesh.attributes.remove(old)
    mesh.materials.clear()
    for material in source.data.materials:mesh.materials.append(material)
    uv=mesh.uv_layers.new(name='Cottage fallback atlas')
    for p in mesh.polygons:
        _,inverse,coords,material=max(donors,key=lambda d:d[0].dot(p.normal))
        for li in p.loop_indices:
            weights=inverse@_screen(mesh.vertices[mesh.loops[li].vertex_index].co)
            # Keep unknown new edges inside their cropped fallback atlas.
            weights=Vector(tuple(max(0,min(1,w)) for w in weights))
            weights/=max(sum(weights),1e-8)
            uv.data[li].uv=sum((coords[i]*weights[i] for i in range(3)),Vector((0,0)))
        p.material_index=material
    bm=bmesh.new()
    try:
        bm.from_mesh(mesh)
        if subdivide:
            bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=3,use_grid_fill=True)
            bmesh.ops.triangulate(bm,faces=list(bm.faces))
        defects={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
                 'degenerate_faces':sum(f.calc_area()<1e-7 for f in bm.faces)}
        if any(defects.values()):raise ValueError(f'Invalid component {obj.name}: {defects}')
        bm.to_mesh(mesh)
    finally:bm.free()
    mesh.update();obj['projection_min_cosine']=.25
    return {'object':obj.name,'source_node':obj['source_node'],'faces':len(mesh.polygons),'validation':defects}


def _add(source,mesh,name,role):
    obj=bpy.data.objects.new(name,mesh);bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():obj[key]=source[key]
    obj[KEY]=TAG;obj['cottage_component_role']=role;obj['projection_min_cosine']=.25
    return obj


def _window(obj,world):
    face=(world[19],world[16],Vector((world[16].x,world[16].y,0)))
    inverse=Matrix([_screen(p) for p in face]).transposed().inverted()
    points=[sum((face[i]*w[i] for i in range(3)),Vector()) for w in
            (inverse@Vector((x,y,1)) for x,y in ((551,1927),(562,1927),(562,1937),(551,1937)))]
    inward=(world[17]-world[16]).normalized()
    vertices=[p+inward*d for d in (-.5,3.5) for p in points]
    mesh=bpy.data.meshes.new('West cottage window cutter')
    mesh.from_pydata(vertices,[],[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    cutter=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(cutter)
    modifier=obj.modifiers.new('Painted extension window recess','BOOLEAN')
    modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
    bpy.context.view_layer.objects.active=obj
    try:bpy.ops.object.modifier_apply(modifier=modifier.name)
    finally:
        bpy.data.objects.remove(cutter,do_unlink=True)
        if not mesh.users:bpy.data.meshes.remove(mesh)


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.objects if o.get(KEY)==TAG and not o.hide_render]
    if existing:
        if {o.get('source_node') for o in existing}!={f'building-{n:03}' for n in (53,54,55,56)}:
            raise ValueError('Incomplete west cottage refinement')
        return {'status':'existing','objects':[o.name for o in existing]}
    previous=[o for o in working.objects if o.type=='MESH' and o.get('asset_group')=='derby-lower-west-cottage' and not o.hide_render]
    sources={}
    for n in (53,54,55,56):
        candidates=[o for o in working.objects if o.type=='MESH' and o.get('source_node')==f'building-{n:03}' and len(o.data.vertices)==20]
        if len(candidates)!=1:raise ValueError(f'Expected one retained baseline for cottage part {n}')
        sources[n]=candidates[0]
    worlds={n:_world(o) for n,o in sources.items()}
    w55,w56=worlds[55][1],worlds[56][1]
    front=(w55[18]+w56[17])/2;back=(w55[17]+w56[18])/2
    direction=back-front;direction.z=0;direction.normalize()
    front-=direction*3;back+=direction*2
    reports=[]
    for n in (55,56):
        source=sources[n];matrix,world=worlds[n]
        outer_front,outer_back=(world[19],world[16]) if n==55 else (world[16],world[19])
        out=outer_front-front;out.z=0;out.normalize()
        eave_front=outer_front-direction*3+out*3.5;eave_back=outer_back+direction*2+out*3.5
        points=[];rows,columns=25,9
        for row in range(rows):
            # Image-space height combines world depth and elevation. Keep the
            # main ridge/eaves level, solving depth from their source outlines.
            # The lower front triangle is a hip, not a sloping main ridge.
            for col in range(columns):
                t=col/(columns-1)
                bulge=1.4*math.sin(math.pi*t)
                height=RIDGE_HEIGHT*(1-t)+EAVE_HEIGHT*t+bulge
                apex=Vector((607,1919,1))
                corner=Vector((651.9553,1969.057,1)) if n==55 else Vector((564.1861,1962.3821,1))
                projected=apex.lerp(corner,t)
                p=Vector((projected.x,-(projected.y+height*COS)/SIN,height))
                rear=back.lerp(eave_back,t)
                rear.z=121.53*(1-t)+87*t+bulge
                p=_level_preserving_projection(p,height)
                rear+=_rear_shift(rear,REAR_OUTLINE[n][col])
                rear=_level_preserving_projection(rear,height)
                u=row/(rows-1)
                p=p.lerp(rear,u)
                # Only the rounded front thatch lip droops. The longitudinal
                # ridge and the remaining 70% of each eave stay horizontal.
                blend=min(1,u/.3)
                droop=17*t*(1-blend*blend*(3-2*blend))
                points.append(_level_preserving_projection(p,p.z-droop))
        top_faces=[]
        for row in range(rows-1):
            for col in range(columns-1):
                a=row*columns+col;top_faces.append((a,a+1,a+1+columns,a+columns))
        lip=front.copy();lip.z=70
        points.append(lip)
        top_faces.append(tuple([len(points)-1]+list(reversed(range(columns)))))
        mesh=_roof_shell(f'West cottage {n} thick thatch',points,top_faces,4)
        obj=_add(source,mesh,'Lower Bailey West Cottage'+(' / East thatch roof' if n==55 else ' / West thatch roof'),'thatch roof')
        reports.append(_finish(obj,source,matrix,world))
        roof_tree=BVHTree.FromPolygons([v.co for v in obj.data.vertices],
                                     [tuple(p.vertices) for p in obj.data.polygons])
        last_row=(rows-1)*columns
        rear_center=points[last_row].lerp(points[0],.16)
        rear_outer=points[last_row+columns-1].lerp(points[columns-1],.16).lerp(rear_center,.06)
        outline=[lip.lerp(points[0],.4),points[0].copy(),rear_center,rear_outer,outer_front.copy()]
        centroid=sum(outline,Vector())/len(outline)
        top=[]
        for edge,a in enumerate(outline):
            b=outline[(edge+1)%len(outline)]
            for step in range(12):
                q=a.lerp(b,step/12).lerp(centroid,.001)
                hit=roof_tree.ray_cast(Vector((q.x,q.y,1000)),Vector((0,0,-1)))[0]
                if hit is None:raise ValueError(f'Cottage wall escaped roof: {n}, {edge}, {step}')
                q.z=hit.z-3.8;top.append(q)
        # Keep wall foundations straight; only their upper edge follows the
        # actual roof underside, closing the hip and rear gable without gaps.
        wall_mesh=_solid(f'West cottage {n} wall shell',top)
        obj=_add(source,wall_mesh,
                 'Lower Bailey West Cottage'+(' / East walls' if n==55 else ' / West walls'),'walls')
        reports.append(_finish(obj,source,matrix,world))
    source=sources[54];matrix,world=worlds[54]
    top=[world[i].copy() for i in (16,17,18,19)]
    for p in top:p.z-=4
    obj=_add(source,_solid('West cottage extension walls',top),'Lower Bailey West Cottage / Extension walls','extension walls')
    bpy.context.view_layer.update()
    reports.append(_finish(obj,source,matrix,world,True))
    center=sum((world[i] for i in (16,17,18,19)),Vector())/4;roof=[]
    for i in (16,19,17,18):
        p=world[i].copy();delta=p-center;delta.z=0;p+=delta.normalized()*3;roof.append(p)
    obj=_add(source,_shell('West cottage lean-to thatch',roof,2,2,4),
             'Lower Bailey West Cottage / Extension thatch eaves','extension roof')
    reports.append(_finish(obj,source,matrix,world))
    source=sources[53];matrix,world=worlds[53]
    porch=next((o for o in previous if o.get('source_node')=='building-053' and
                (o.get('cottage_refinement') or o.get('cottage_component_role')=='porch body')),None)
    if porch is None:raise ValueError('Apply the audited porch arch refinement before the cottage shell')
    mesh=porch.data.copy();mesh.transform(porch.matrix_world)
    obj=_add(source,mesh,'Lower Bailey West Cottage / Porch with recessed doorway','porch body')
    reports.append(_finish(obj,source,matrix,world))
    vertices=[];faces=[]
    def beam(a,b,width,height):
        axis=(b-a).normalized();side=axis.cross(Vector((0,0,1))).normalized()*width/2
        up=Vector((0,0,height/2));center=(a+b)/2;length=(b-a)/2;start=len(vertices)
        vertices.extend(center+x*length+y*side+z*up for z in (-1,1) for y in (-1,1) for x in (-1,1))
        faces.extend(tuple(start+i for i in f) for f in ((0,2,3,1),(4,5,7,6),(0,1,5,4),(2,6,7,3),(0,4,6,2),(1,3,7,5)))
    lf,rf,lb,rb=(world[i] for i in (18,19,17,16))
    for t in (.04,.22,.40,.58,.76,.94):
        beam(lf.lerp(rf,t)+Vector((0,0,1.3)),lb.lerp(rb,t)+Vector((0,0,1.3)),1.6,2.6)
    for t in (.15,.5,.85):
        beam(lf.lerp(lb,t)+Vector((0,0,3)),rf.lerp(rb,t)+Vector((0,0,3)),1.2,1.2)
    mesh=bpy.data.meshes.new('West cottage porch roof lattice');mesh.from_pydata(vertices,[],faces)
    obj=_add(source,mesh,'Lower Bailey West Cottage / Porch exposed roof timbers','porch roof timbers')
    reports.append(_finish(obj,source,matrix,world))
    for obj in previous:
        obj.hide_render=True;obj.hide_set(True);obj['superseded_cottage_refinement']=TAG
    bpy.context.view_layer.update()
    return {'status':'refined','version':TAG,'parts':reports,'projection_min_cosine':.25,
            'roof_thickness':4,'side_wall_inset_fraction':.06,'rear_wall_inset_fraction':.16,
            'rear_roof_source_outline':REAR_OUTLINE,
            'main_ridge_height':RIDGE_HEIGHT,'main_eave_height':EAVE_HEIGHT,
            'front_lip_length_fraction':.3,'front_lip_height':70,
            'hip_apex_source_pixel':[607,1919],
            'limitations':['Concealed walls need multiview completion; unseen openings are not invented.',
                           'Porch rails have geometry; the underlying roof closes unobserved interior.']}
