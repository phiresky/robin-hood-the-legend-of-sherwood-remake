"""Source-measured closed pointed window relief, preserving reveal semantics."""
import bpy,bmesh
from mathutils import Vector


def _refine_pointed_window(node, center, normal, bottom, tag):
    found=[o for o in bpy.data.collections['Derby Working'].all_objects
           if o.type=='MESH' and not o.hide_render and o.get('source_node')==f'building-{node:03d}']
    if len(found)!=1:raise ValueError('Expected one principal Hall facade')
    obj=found[0]
    if obj.get('east_hall_pointed_window')==tag:return {'status':'already-refined'}
    normal=Vector(normal).normalized()
    across=Vector((-normal.y,normal.x,0))
    center=Vector(center)
    depth=2.0
    contour=[(-24.,bottom),(24.,bottom),(24.,247.)]
    for a,b,c in [((24.,247.),(19.,275.),(0.,313.)),((0.,313.),(-19.,275.),(-24.,247.))]:
        for i in range(1,13):
            t=i/12;contour.append(((1-t)**2*a[0]+2*(1-t)*t*b[0]+t*t*c[0],
                                 (1-t)**2*a[1]+2*(1-t)*t*b[1]+t*t*c[1]))
    def uv(p):return ((p-center).dot(across),p.z)
    def split(polygon,a,b):
        inside,outside=[],[]
        def value(p):
            u,z=uv(p);return (b[0]-a[0])*(z-a[1])-(b[1]-a[1])*(u-a[0])
        for p,q in zip(polygon,polygon[1:]+polygon[:1]):
            fp,fq=value(p),value(q)
            (inside if fp>=-1e-6 else outside).append(p)
            if (fp>1e-6 and fq<-1e-6) or (fp<-1e-6 and fq>1e-6):
                crossing=p+(q-p)*(fp/(fp-fq));inside.append(crossing);outside.append(crossing)
        def clean(points):
            result=[]
            for p in points:
                if not result or (p-result[-1]).length>1e-5:result.append(p)
            if len(result)>1 and (result[0]-result[-1]).length<1e-5:result.pop()
            return result if len(result)>=3 else []
        return clean(inside),clean(outside)
    def area(polygon):
        if len(polygon)<3:return 0.
        return sum((polygon[i]-polygon[0]).cross(polygon[i+1]-polygon[0]).length/2 for i in range(1,len(polygon)-1))
    points=[obj.matrix_world@v.co for v in obj.data.vertices]
    bm=bmesh.new();bm.from_mesh(obj.data);old_open=sum(not e.is_manifold for e in bm.edges);bm.free()
    output=[];inner=[];selected=0
    for face in obj.data.polygons:
        polygon=[points[i] for i in face.vertices]
        n=obj.matrix_world.to_3x3()@face.normal
        if n.dot(normal)<.995 or max(abs((p-center).dot(normal)) for p in polygon)>.3:
            output.append(polygon);continue
        remaining=polygon;fragments=[]
        for a,b in zip(contour,contour[1:]+contour[:1]):
            if not remaining:break
            remaining,out=split(remaining,a,b)
            if area(out)>1e-6:fragments.append(out)
        if area(remaining)>1e-6:
            selected+=1;output.extend(fragments);inner.append(remaining)
        else:output.append(polygon)
    if not inner:raise ValueError('Pointed window does not intersect the reviewed facade plane')
    # Shared subdivisions of the displaced panel move together; only boundary
    # edges need a masonry reveal connecting them to the original facade.
    def key(p):return tuple(round(float(v),3) for v in p)
    edges={}
    for polygon in inner:
        output.append([p-normal*depth for p in polygon])
        for p,q in zip(polygon,polygon[1:]+polygon[:1]):
            edges.setdefault(tuple(sorted((key(p),key(q)))),[]).append((p,q))
    for pair in edges.values():
        if len(pair)==1:
            p,q=pair[0];output.append([p,q,q-normal*depth,p-normal*depth])
    vertices=[];faces=[];lookup={}
    for polygon in output:
        if area(polygon)<1e-6:continue
        face=[]
        for p in polygon:
            k=key(p)
            if k not in lookup:lookup[k]=len(vertices);vertices.append(p)
            face.append(lookup[k])
        if len(set(face))>=3:faces.append(face)
    # Split collinear boundary points in adjacent faces before welding.
    stitched=[]
    for face in faces:
        result=[]
        for a,b in zip(face,face[1:]+face[:1]):
            if a==b:continue
            result.append(a);delta=vertices[b]-vertices[a];cuts=[]
            if delta.length_squared<1e-10:continue
            for i,p in enumerate(vertices):
                if i in (a,b):continue
                t=(p-vertices[a]).dot(delta)/delta.length_squared
                if 1e-6<t<1-1e-6 and (vertices[a]+delta*t-p).length<.001:cuts.append((t,i))
            result.extend(i for t,i in sorted(cuts))
        if len(set(result))>=3:stitched.append(result)
    inverse=obj.matrix_world.inverted();mesh=bpy.data.meshes.new(obj.data.name+' pointed window')
    mesh.from_pydata([inverse@p for p in vertices],[],stitched);mesh.update()
    for material in obj.data.materials:mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    invalid=sum(f.calc_area()<1e-7 for f in bm.faces);opened=sum(not e.is_manifold for e in bm.edges)
    if invalid or opened>old_open:raise ValueError(f'Window topology regressed: degenerate{invalid},open{old_open}->{opened}')
    bm.to_mesh(mesh);bm.free();obj.data=mesh
    obj['east_hall_pointed_window']=tag;obj['east_hall_window_depth_inferred']=depth
    return {'source_node':f'building-{node:03d}','closed_glass_panel':True,'depth_inferred':depth,
            'modified_front_faces':selected,'open_edges_before':old_open,'open_edges_after':opened,'degenerate':invalid,
            'window_local_contour':contour,'adjacent_open_casement_preserved':True}


def refine_closed_pointed_window():
    return _refine_pointed_window(183,(1478,-2334.39+(.6560574/.7547109)*(1478-1480),0),
                                  (.6560574,-.7547109,0),150.,'hall-closed-pointed-window-v1')


def refine_casement_transom():
    """Only the fixed glazed arch above the open leaf is recessed."""
    return _refine_pointed_window(189,(1540,-2282.144,0),(.6505046,-.7595022,0),
                                  244.,'hall-fixed-casement-transom-v1')
