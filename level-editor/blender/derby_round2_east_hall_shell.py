"""Close missing masonry undersides and retain the source-visible open leaf."""
import bpy,bmesh
from mathutils import Vector


def _object(node):
    found=[o for o in bpy.data.collections['Derby Working'].all_objects
           if o.type=='MESH' and not o.hide_render and o.get('source_node')==f'building-{node:03d}']
    if len(found)!=1:raise ValueError(f'Expected one Hall part{node}')
    return found[0]


def close_wall_undersides():
    """Cap masonry, never the casement aperture below its transom.

    Wall183 has one missing foundation contour spanning ground to the inside
    floor underside. Extend its interior skirt down to the same foundation,
    then cap the thin L-shaped wall footprint.189 has only the planar underside
    of the fixed upper transom missing, at241.104; filling it leaves the opening
    below and the revealed room untouched.
    """
    report=[]
    for node in (183,189):
        obj=_object(node);tag='reviewed-foundation-and-transom-caps-v1'
        if obj.get('east_hall_shell_caps')==tag:
            report.append({'source_node':obj['source_node'],'status':'already-capped'});continue
        bm=bmesh.new();bm.from_mesh(obj.data)
        for v in bm.verts:v.co=obj.matrix_world@v.co
        edges=[e for e in bm.edges if e.is_boundary];before=len(edges)
        if before!=(36 if node==183 else 8):raise ValueError(f'Reaudit changed boundary topology{node}:{before}')
        if node==183:
            ground={}
            def down(v):
                if abs(v.co.z)<.001:return v
                if v not in ground:ground[v]=bm.verts.new((v.co.x,v.co.y,0))
                return ground[v]
            for edge in edges:
                a,b=edge.verts;poly=[a,b,down(b),down(a)];clean=[]
                for v in poly:
                    if v not in clean:clean.append(v)
                if len(clean)>=3:
                    area=(clean[1].co-clean[0].co).cross(clean[2].co-clean[0].co).length
                    if area>1e-6:bm.faces.new(clean)
            bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.002)
        boundary=[e for e in bm.edges if e.is_boundary]
        expected=0 if node==183 else 241.1042
        if any(abs(v.co.z-expected)>.003 for e in boundary for v in e.verts):raise ValueError('Cap boundary is not the reviewed underside plane')
        filled=bmesh.ops.holes_fill(bm,edges=boundary,sides=0)['faces']
        area=sum(f.calc_area() for f in filled)
        if node==183 and not 1000<area<10000:raise ValueError(f'Unexpected foundation footprint{area}')
        if node==189 and not 100<area<1500:raise ValueError(f'Unexpected transom footprint{area}')
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        opened=sum(not e.is_manifold for e in bm.edges);invalid=sum(f.calc_area()<1e-7 for f in bm.faces)
        if opened or invalid:raise ValueError(f'Cap remains invalid{node}:open{opened},degenerate{invalid}')
        inverse=obj.matrix_world.inverted()
        for v in bm.verts:v.co=inverse@v.co
        mesh=obj.data.copy();bm.to_mesh(mesh);bm.free();obj.data=mesh;obj['east_hall_shell_caps']=tag
        report.append({'source_node':obj['source_node'],'open_before':before,'open_after':opened,'degenerate':invalid,'cap_area':area})
    return report


def add_open_casement_leaf():
    """Add the observed right-hand leaf, open inward beside the clear aperture.

    The visible free edge is around source x1538,y1110..1175, the hinge around
    x1554,y1104..1187. Its upper free corner is occluded by the fixed transom.
    The plan angle/depth is inferred from this projection and the hinge plane.
    """
    obj=_object(189);tag='hall-observed-open-glazed-leaf-v1'
    if obj.get('east_hall_open_leaf')==tag:return {'status':'already-refined'}
    hinge=Vector((1554,-2270.153,0));free=Vector((1538,-2249.50,0))
    delta=free-hinge;normal=Vector((delta.y,-delta.x,0)).normalized();thickness=.6
    front=[hinge+Vector((0,0,141)),free+Vector((0,0,141)),free+Vector((0,0,241.1042)),hinge+Vector((0,0,241.1042))]
    points=front+[p+normal*thickness for p in front]
    faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
    bm=bmesh.new();bm.from_mesh(obj.data);inverse=obj.matrix_world.inverted()
    verts=[bm.verts.new(inverse@p) for p in points]
    for face in faces:bm.faces.new([verts[i] for i in face])
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));invalid=sum(f.calc_area()<1e-7 for f in bm.faces)
    opened=sum(not e.is_manifold for e in bm.edges)
    if opened or invalid:raise ValueError(f'Leaf topology invalid:open{opened},degenerate{invalid}')
    mesh=obj.data.copy();bm.to_mesh(mesh);bm.free();obj.data=mesh;obj['east_hall_open_leaf']=tag
    return {'source_node':'building-189','closed_thin_glazing_leaf':True,'leaf_is_open':True,'width':delta.length,
            'thickness':thickness,'bottom':141,'top':241.1042,'plan_depth_inferred':True,'nonmanifold':opened,'degenerate':invalid}
