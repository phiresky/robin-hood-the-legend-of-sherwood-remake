"""Trace the approach outcrop's angular silhouette instead of two box wedges.

Reference pixels define the visible outline. Heights are estimated within the
existing 29.3-unit envelope; hidden rock depth is not recoverable from the art.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

TAG='south-approach-traced-outcrop-v1'
OUTLINES={
    47: [(760,2557,29),(748,2566,16),(731,2581,2),(741,2590,0),(760,2594,0)],
    48: [(760,2557,29),(765,2556,29),(774,2564,20),(789,2577,7),(782,2588,1),(760,2594,0)],
}
CREST=(760,2573,18)


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.all_objects if o.get('approach_stone_refinement')==TAG]
    if existing:
        if len(existing)!=2:raise ValueError('Incomplete approach stone refinement')
        return {'status':'existing','parts':2}
    bpy.context.view_layer.update()
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    def world(point):
        x,y,z=point
        return Vector((x,-(y+z*cosine)/sine,z))
    def screen(point):return Vector((point.x,-point.y*sine-point.z*cosine,1))
    reports=[]
    for number,outline in OUTLINES.items():
        source=next(o for o in working.all_objects if o.get('source_node')==f'building-{number:03d}' and not o.hide_render)
        original=[source.matrix_world@v.co for v in source.data.vertices]
        top=[world(p) for p in outline];count=len(top)
        verts=top+[Vector((p.x,p.y,-.5)) for p in top]+[world(CREST)]
        center=len(verts)-1
        faces=[(i,(i+1)%count,center) for i in range(count)]
        faces += [tuple(reversed(range(count,count*2)))]
        faces += [(i,count+i,count+(i+1)%count,(i+1)%count) for i in range(count)]
        mesh=bpy.data.meshes.new(source.name+' traced facets');mesh.from_pydata(verts,[],faces);mesh.update()
        uv=mesh.uv_layers.new(name='UVMap');mappings=[]
        for face in source.data.polygons:
            matrix=Matrix([screen(original[i]) for i in face.vertices]).transposed()
            if abs(matrix.determinant())>1e-8:mappings.append((face,matrix.inverted(),[source.data.uv_layers.active.data[i].uv.copy() for i in face.loop_indices]))
        for polygon in mesh.polygons:
            centerpoint=sum((verts[i] for i in polygon.vertices),Vector())/len(polygon.vertices)
            def score(entry):
                face=entry[0];normal=(source.matrix_world.to_3x3()@face.normal).normalized();facecenter=sum((original[i] for i in face.vertices),Vector())/len(face.vertices)
                return abs(normal.dot(polygon.normal))-abs((centerpoint-facecenter).dot(normal))*.02
            face,inverse,triuv=max(mappings,key=score)
            for loop in polygon.loop_indices:
                weights=inverse@screen(verts[mesh.loops[loop].vertex_index])
                weights=Vector(tuple(max(0,min(1,w)) for w in weights));weights/=sum(weights)
                uv.data[loop].uv=sum((triuv[i]*weights[i] for i in range(3)),Vector((0,0)))
        for material in source.data.materials:mesh.materials.append(material)
        bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bmesh.ops.triangulate(bm,faces=list(bm.faces))
        bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=2,use_grid_fill=True);bmesh.ops.triangulate(bm,faces=list(bm.faces))
        defects={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),'degenerate_faces':sum(f.calc_area()<1e-7 for f in bm.faces)}
        if any(defects.values()):bm.free();raise ValueError(f'{number}: {defects}')
        bm.to_mesh(mesh);bm.free()
        obj=bpy.data.objects.new(source.name+' / Traced outcrop',mesh);working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
        for key in source.keys():obj[key]=source[key]
        obj['approach_stone_refinement']=TAG;source.hide_render=source.hide_viewport=True
        reports.append({'source':obj['source_node'],'faces':len(mesh.polygons),'validation':defects})
    return {'status':'refined','parts':reports,'reference_outline_pixels':OUTLINES,'limitations':['Rock depth and hidden rear facets are inferred within the previous height envelope','Fine cracks remain part of the source texture']}
