"""Replace the timber pile's two wedge proxies with closed staggered logs.

The source shows round cut ends and bark cylinders. Log count and hidden rear
ends are authored approximations within the existing measured pile envelope.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = 'east-bailey-round-log-stack-v1'


def refine():
    working = bpy.data.collections['Derby Working']
    existing = [o for o in working.all_objects if o.get('timber_log_refinement') == TAG]
    if existing:
        if len(existing) != 2:
            raise ValueError('Incomplete timber refinement')
        return {'status': 'existing', 'logs': sum(o['log_count'] for o in existing)}
    bpy.context.view_layer.update()
    sources = {n: next(o for o in working.all_objects if o.get('source_node') == f'building-{n}' and not o.hide_render) for n in (107,108)}
    world = {n: [o.matrix_world @ v.co for v in o.data.vertices] for n,o in sources.items()}
    a, b = world[108][15].copy(), world[107][12].copy()
    a.z=b.z=0
    across = b-a; width=across.length; across.normalize()
    axis=world[108][12]-world[108][15]; axis.z=0
    length=axis.length; axis.normalize()
    ridge=(world[108][14]-a).dot(across)
    radius=2.8; pitch=radius*2
    row_height=math.sqrt((radius*2)**2-(pitch*.5)**2)
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    def screen(v): return Vector((v.x,-v.y*sine-v.z*cosine,1))
    outputs={n: {'verts':[], 'faces':[], 'logs':0} for n in sources}
    for row in range(6):
        z=radius+row*row_height
        offset=radius+(pitch*.5 if row%2 else 0)
        for column in range(20):
            distance=offset+column*pitch
            if distance+radius>width: break
            roof=14.65+(29.70-14.65)*distance/ridge if distance<=ridge else 29.70+(18.72-29.70)*(distance-ridge)/(width-ridge)
            if z+radius>roof+.3: continue
            n=108 if distance<ridge else 107
            data=outputs[n]; center=a+across*distance+Vector((0,0,z))
            start=len(data['verts'])
            for ring in range(7):
                for side in range(12):
                    angle=2*math.pi*side/12
                    data['verts'].append(center+axis*(ring*length/6)+across*(radius*math.cos(angle))+Vector((0,0,radius*math.sin(angle))))
            data['faces'].extend([tuple(start+i for i in reversed(range(12))),tuple(start+72+i for i in range(12))])
            for ring in range(6):
                base=start+12*ring
                data['faces'].extend((base+i,base+(i+1)%12,base+12+(i+1)%12,base+12+i) for i in range(12))
            data['logs']+=1
    report=[]
    for n,data in outputs.items():
        source=sources[n]; vertices=data['verts']
        mesh=bpy.data.meshes.new(source.name+' individual logs'); mesh.from_pydata(vertices,[],data['faces']); mesh.update()
        uv=mesh.uv_layers.new(name='UVMap')
        mappings=[]
        for face in source.data.polygons:
            matrix=Matrix([screen(world[n][i]) for i in face.vertices]).transposed()
            if abs(matrix.determinant())>1e-7:
                mappings.append((face,matrix.inverted(),[source.data.uv_layers.active.data[i].uv.copy() for i in face.loop_indices]))
        for polygon in mesh.polygons:
            center=sum((vertices[i] for i in polygon.vertices),Vector())/len(polygon.vertices)
            def score(entry):
                face=entry[0]; normal=(source.matrix_world.to_3x3()@face.normal).normalized()
                fc=sum((world[n][i] for i in face.vertices),Vector())/len(face.vertices)
                return abs(normal.dot(polygon.normal))-abs((center-fc).dot(normal))*.02
            # Hidden cylinder flanks have no recoverable source pixels. Sample
            # the middle of the photographed bark instead of extrapolating the
            # old wedge UVs into adjacent masonry and bright ground texels.
            face,inverse,triangle_uv=next(entry for entry in mappings if entry[0].index == 6)
            bark_uv=sum(triangle_uv,Vector((0,0)))/3
            for loop in polygon.loop_indices:
                uv.data[loop].uv=bark_uv
        for material in source.data.materials: mesh.materials.append(material)
        bm=bmesh.new(); bm.from_mesh(mesh); bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces)); bmesh.ops.triangulate(bm,faces=list(bm.faces))
        defects={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),'degenerate_faces':sum(f.calc_area()<1e-7 for f in bm.faces)}
        if any(defects.values()): bm.free(); raise ValueError(str(defects))
        bm.to_mesh(mesh); bm.free()
        obj=bpy.data.objects.new(source.name+' / Individual round logs',mesh); working.objects.link(obj); obj.parent=source.parent; obj.matrix_world=Matrix.Identity(4)
        for key in source.keys(): obj[key]=source[key]
        obj['timber_log_refinement']=TAG; obj['log_count']=data['logs']; source.hide_render=source.hide_viewport=True
        report.append({'source':source['source_node'],'logs':data['logs'],'validation':defects})
    return {'status':'refined','parts':report,'log_radius':radius,'log_length':length,'limitations':['Exact hidden log count and rear ends are not visible in the reference','Covered rear bark retains source-atlas fallback']}
