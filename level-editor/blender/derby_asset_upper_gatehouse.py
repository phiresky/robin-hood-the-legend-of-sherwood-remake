"""Upper Gatehouse source-measured battlements and pointed masonry passage.

Keep the narrow passage closure and patch-controlled upper chamber separate.
The render recipes do not change collision records or inferred door state.
"""
import math
import runpy
from pathlib import Path
import bpy
import bmesh
from mathutils import Matrix, Vector

ASSET = 'derby-upper-gatehouse'
TAG = 'upper-gatehouse-crenels-arch-v1'
REAR_GAPS = [(571,589),(609,628),(649,668),(689,708),(729,742)]
RECIPES = {
    250: [(4,0,11,2,[(505,520),(536,552)],25)],
    256: [(0,7,22,16,REAR_GAPS,27)],
    257: [(4,3,8,2,[(590,606),(625,640),(659,675),(693,710)],27)],
    263: [(0,2,15,19,[(590,606)],27)],
    264: [(4,3,8,2,REAR_GAPS,27)],
}


def _validation(obj):
    bm=bmesh.new();bm.from_mesh(obj.data)
    result={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
            'degenerate_faces':sum(f.calc_area()<1e-7 for f in bm.faces)}
    bm.free()
    if any(result.values()):raise ValueError(f'{obj.name}: {result}')
    return result


def _close(source):
    points=[source.matrix_world@v.co for v in source.data.vertices]
    bottom=min(p.z for p in points)
    caps=[]
    for face in source.data.polygons:
        a,b,c=[points[i] for i in face.vertices[:3]]
        normal=(b-a).cross(c-a).normalized()
        if abs(normal.z)>.03 and min(points[i].z for i in face.vertices)>bottom+.1:
            caps.append(face)
    if not caps:raise ValueError(f'No upper surface on {source.name}')
    bm=bmesh.new();verts={}
    for face in caps:
        vs=[]
        for i in face.vertices:
            if i not in verts:verts[i]=bm.verts.new(points[i])
            vs.append(verts[i])
        bm.faces.new(vs)
    bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.6)
    bm.verts.ensure_lookup_table();bm.verts.index_update()
    top=[v.co.copy() for v in bm.verts];n=len(top)
    polygons=[tuple(v.index for v in f.verts) for f in bm.faces]
    faces=list(polygons)+[tuple(i+n for i in reversed(f)) for f in polygons]
    for edge in bm.edges:
        if edge.is_boundary:
            a,b=(v.index for v in edge.verts);faces.append((a,b,b+n,a+n))
    bm.free()
    vertices=top+[Vector((p.x,p.y,bottom)) for p in top]
    donors=[]
    for face in source.data.polygons:
        a,b,c=[points[i] for i in face.vertices[:3]]
        normal=(b-a).cross(c-a).normalized()
        projection=Matrix([Vector((p.x,-p.y*.573576436-p.z*.819152044,1)) for p in (a,b,c)]).transposed()
        if abs(projection.determinant())>.001:donors.append((face.index,normal,(a+b+c)/3))
    mappings=[]
    for face in faces:
        a,b,c=[vertices[i] for i in face[:3]];normal=(b-a).cross(c-a).normalized()
        center=sum((vertices[i] for i in face),Vector())/len(face)
        mappings.append(max(donors,key=lambda d:abs(d[1].dot(normal))*10000-(d[2]-center).length)[0])
    mapped=runpy.run_path(str(Path(__file__).with_name('derby_asset_lower_east_cottage.py')))['_mapped_mesh']
    mesh=mapped(source,vertices,faces,mappings)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(source.name+' / reviewed shell',mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith('reprojection_'):obj[key]=source[key]
    _validation(obj)
    source.hide_render=True;source.hide_set(True)
    return obj


def _arch_cutter(front):
    points=[front.matrix_world@v.co for v in front.data.vertices]
    left,right=points[4],points[3]
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    # Hand-traced inner edge of the lower pointed arch in covered artwork.
    traced=[(617,1511),(620,1504),(627,1497),(637,1490),(647,1485),
            (655,1483),(664,1486),(675,1490),(687,1497),(699,1505),(707,1515)]
    profile=[]
    for x,screen_y in traced:
        y=left.y+(right.y-left.y)*(x-left.x)/(right.x-left.x)
        profile.append(Vector((x,y-8,(-y*sine-screen_y)/cosine)))
    profile.extend([Vector((profile[-1].x,profile[-1].y,-3)),
                    Vector((profile[0].x,profile[0].y,-3))])
    count=len(profile);vertices=profile+[p+Vector((0,180,0)) for p in profile]
    faces=[tuple(range(count)),tuple(reversed(range(count,count*2)))]
    faces += [(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
    mesh=bpy.data.meshes.new('Upper gate measured arch cutter');mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(obj)
    _validation(obj)
    return obj


def _finish_projection_cells(obj):
    """Keep new reveals on stone and refresh visible cells in the layer pass."""
    helper=runpy.run_path(str(Path(__file__).with_name('derby_asset_lower_east_curtain.py')))
    material=helper['_stone_donor']()
    index=len(obj.data.materials);obj.data.materials.append(material)
    uv=obj.data.uv_layers.get('UVMap') or obj.data.uv_layers.new(name='UVMap')
    fallback=obj.data.attributes.get('reprojection_fallback_material')
    if fallback is None:fallback=obj.data.attributes.new('reprojection_fallback_material','INT','FACE')
    for face in obj.data.polygons:
        face.material_index=index;fallback.data[face.index].value=index
        axis=max(range(3),key=lambda i:abs(face.normal[i]));axes=[i for i in range(3) if i!=axis]
        for loop in face.loop_indices:
            p=obj.data.vertices[obj.data.loops[loop].vertex_index].co
            uv.data[loop].uv=(p[axes[0]]/80,p[axes[1]]/45)
    bm=bmesh.new();bm.from_mesh(obj.data)
    bmesh.ops.triangulate(bm,faces=list(bm.faces))
    bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=7,use_grid_fill=True,smooth=0)
    bm.to_mesh(obj.data);bm.free()


def refine():
    bpy.context.view_layer.update();working=bpy.data.collections['Derby Working']
    before_ids={o.get('source_node') for o in working.objects
                if o.type=='MESH' and not o.hide_render and o.get('source_node')}
    originals=[o for o in working.objects if o.type=='MESH' and not o.hide_render
               and o.get('asset_group')==ASSET and not o.get('step_count')]
    if len(originals)!=23:raise ValueError(f'Expected 23 original components, got {len(originals)}')
    if any(o.get('upper_gatehouse_recipe') for o in originals):
        if not all(o.get('upper_gatehouse_recipe')==TAG for o in originals):
            raise ValueError('Incomplete upper gatehouse refinement')
        return {'asset':ASSET,'status':'already-refined'}
    sources={int(o['source_node'].split('-')[1]):o for o in originals}
    cutter=_arch_cutter(sources[257])
    cut=runpy.run_path(str(Path(__file__).with_name('derby_asset_east_hall.py')))['_refine']
    report=[]
    try:
        for number,source in sources.items():
            original=source;notches=0
            obj=_close(source)
            if number in RECIPES:
                points=[source.matrix_world@v.co for v in source.data.vertices]
                current=[v.co for v in obj.data.vertices]
                converted=[]
                for *indices,gaps,depth in RECIPES[number]:
                    new=[min(range(len(current)),key=lambda j:(current[j]-points[i]).length_squared) for i in indices]
                    converted.append((*new,gaps,depth))
                item=cut(obj,converted);notches=item['notches'];obj=bpy.data.objects[obj['replaced_by']]
            if number in (249,254,255,259,260,263,264):
                modifier=obj.modifiers.new('Pointed lower arch opening','BOOLEAN')
                modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
                bpy.context.view_layer.objects.active=obj;bpy.ops.object.modifier_apply(modifier=modifier.name)
            elif number==267:
                # Preserve the separate thin closure; shape its top to the arch.
                modifier=obj.modifiers.new('Closure fitted within arch','BOOLEAN')
                modifier.operation='INTERSECT';modifier.solver='EXACT';modifier.object=cutter
                bpy.context.view_layer.objects.active=obj;bpy.ops.object.modifier_apply(modifier=modifier.name)
            if len(obj.data.polygons)==0:raise ValueError(f'Arch erased component {number}')
            obj['upper_gatehouse_recipe']=TAG
            obj['crenellation_notches']=notches
            if number==267:obj['part_name']='Gate passage closure'
            _finish_projection_cells(obj)
            report.append({'source_node':obj['source_node'],'notches':notches,**_validation(obj)})
            original.hide_render=True;original.hide_set(True)
    finally:
        mesh=cutter.data;bpy.data.objects.remove(cutter,do_unlink=True);bpy.data.meshes.remove(mesh)
    after_ids={o.get('source_node') for o in working.objects
               if o.type=='MESH' and not o.hide_render and o.get('source_node')}
    if before_ids!=after_ids:raise ValueError('Upper Gatehouse changed map source ownership')
    return {'asset':ASSET,'parts':report,'mesh_crenel_cuts':sum(r['notches'] for r in report),
            'physical_crenels':11, 'source_ids_preserved':len(after_ids),
            'note':'Overlapping rear/front wall pieces are both cut; closure remains separate.'}
