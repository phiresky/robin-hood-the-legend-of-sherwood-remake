"""Refine source part 069: a lean-to shelter beneath the east wall stair.

The historical asset identifier says landing, but the covered reference shows
a sloping plank roof over timber-and-plaster walls. Retain its stable group and
collision identifier while making the roof and closed body separately named.
"""
import bpy
import bmesh
from mathutils import Matrix, Vector
import math

TAG='east-wall-lean-to-v1'


def _project(point):
    return Vector((point.x,-point.y*math.sin(math.radians(35))
                   -point.z*math.cos(math.radians(35)),1))


def _piece(source,top,bottom,name,kind):
    mesh=bpy.data.meshes.new(name)
    mesh.from_pydata(top+bottom,[],[(0,1,2,3),(7,6,5,4),
                                 (0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bad=sum(not edge.is_manifold for edge in bm.edges)
    degenerate=sum(face.calc_area()<1e-8 for face in bm.faces)
    bm.to_mesh(mesh);bm.free()
    if bad or degenerate:raise RuntimeError('Invalid shelter '+kind)
    uv=mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    points=[source.matrix_world@v.co for v in source.data.vertices]
    normal_matrix=source.matrix_world.to_3x3().inverted().transposed()
    donors=[]
    for face in source.data.polygons:
        matrix=Matrix([_project(points[i]) for i in face.vertices]).transposed()
        if abs(matrix.determinant())<1e-7:continue
        donors.append(((normal_matrix@face.normal).normalized(),matrix.inverted(),
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices],face.index))
    for material in source.data.materials:mesh.materials.append(material)
    attr=mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    backup=source.data.attributes.get('reprojection_fallback_material')
    for face in mesh.polygons:
        # Thin fascia and underside have no independent image evidence: retain
        # the adjoining plank roof donor, rather than sampling unrelated walls.
        candidates=[d for d in donors if d[3]>=8] if kind=='roof' else donors
        normal,inverse,coords,index=max(candidates,key=lambda d:d[0].dot(face.normal))
        material=backup.data[index].value if backup else source.data.polygons[index].material_index
        face.material_index=material;attr.data[face.index].value=material
        for li in face.loop_indices:
            point=mesh.vertices[mesh.loops[li].vertex_index].co.copy()
            if kind=='roof' and mesh.loops[li].vertex_index>=4:point.z+=3.2
            weights=inverse@_project(point)
            uv.data[li].uv=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
    obj=bpy.data.objects.new(name,mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():obj[key]=source[key]
    obj['east_wall_landing_refinement']=TAG
    obj['asset_name']='East Wall Lean-to Shelter'
    obj['part_name']='Lean-to shelter'
    obj['shelter_component']=kind
    return {'object':obj.name,'component':kind,'nonmanifold_edges':bad,'degenerate_faces':degenerate}


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.objects if o.get('east_wall_landing_refinement')==TAG]
    if existing:
        if len(existing)!=2:raise RuntimeError('Incomplete east-wall shelter refinement')
        return {'reused':True,'objects':[o.name for o in existing]}
    candidates=[o for o in working.objects if o.type=='MESH'
                and o.get('source_node')=='building-069' and not o.hide_render]
    if len(candidates)!=1:raise RuntimeError('Expected one part 069 baseline')
    source=candidates[0]
    if len(source.data.vertices)!=20:raise ValueError('Shelter topology changed; re-audit roof corners')
    bpy.context.view_layer.update()
    top=[source.matrix_world@source.data.vertices[i].co for i in (16,17,18,19)]
    roof_bottom=[p-Vector((0,0,3.2)) for p in top]
    foundation=min((source.matrix_world@v.co).z for v in source.data.vertices)
    bottom=[Vector((p.x,p.y,foundation)) for p in top]
    body=_piece(source,roof_bottom,bottom,'East Wall Lean-to Shelter / timber and plaster walls','body')
    roof=_piece(source,top,roof_bottom,'East Wall Lean-to Shelter / plank roof','roof')
    source['east_wall_landing_baseline']=TAG
    source.hide_render=True;source.hide_set(True)
    return {'objects':[body,roof],'source_node':'building-069',
            'source_identification':'Timber and plaster lean-to beneath the wall access stair; not a landing.',
            'preserved_group':'derby-east-wall-landing','roof_thickness':3.2,
            'remaining':'Timber-frame relief and concealed rear/underside artwork remain unverified.'}
