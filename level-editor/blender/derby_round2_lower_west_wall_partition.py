"""Partition the long wall walk at its authored middle junction.

Existing surfaces retain world position, interpolated UVs and material indices.
Only neutral internal cut caps are added; they are explicitly marked synthetic
interfaces. The two disjoint components retain canonical source_node045, with
distinct projection_component IDs for component-aware ownership routing.
"""
import math

import bpy
import bmesh
from mathutils import Vector

SECTIONS = {
    'north': ('derby-lower-west-curtain-north', 'Lower Bailey West Curtain North Wall Walk', {25}),
    'south': ('derby-lower-west-curtain-south', 'Lower Bailey West Curtain South Wall Walk', {23,42}),
}


def _half(source, side, plane_co, plane_no, neutral):
    mesh=source.data.copy()
    mesh.name=source.data.name+' / '+side+' exact partition'
    bm=bmesh.new();bm.from_mesh(mesh)
    origin=bm.faces.layers.int.new('partition_origin_face')
    for face in bm.faces:face[origin]=face.index
    bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),
        dist=.001,plane_co=plane_co,plane_no=plane_no,
        clear_outer=side=='north',clear_inner=side=='south')
    bmesh.ops.dissolve_degenerate(bm,dist=1e-6,edges=list(bm.edges))
    boundary=[e for e in bm.edges if e.is_boundary]
    if not boundary or any(abs((v.co-plane_co).dot(plane_no))>.001 for e in boundary for v in e.verts):
        raise ValueError('Unexpected partition opening outside the reviewed middle plane')
    cap_index=len(mesh.materials);mesh.materials.append(neutral)
    caps=bmesh.ops.holes_fill(bm,edges=boundary,sides=0)['faces']
    for face in caps:
        face[origin]=-1
        face.material_index=cap_index
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bad=sum(not e.is_manifold for e in bm.edges)
    deg=sum(f.calc_area()<1e-8 for f in bm.faces)
    volume=bm.calc_volume(signed=True)
    if bad or deg or volume<=0:
        raise ValueError(f'Invalid closed section {side}: {bad}, {deg}, {volume}')
    bm.to_mesh(mesh);bm.free();mesh.update()
    obj=source.copy();obj.data=mesh
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.matrix_world=source.matrix_world.copy()
    obj['projection_component']='west-wall-walk-'+side
    obj['reprojection_partition_cap_faces']=[p.index for p in mesh.polygons
        if mesh.attributes['partition_origin_face'].data[p.index].value<0]
    obj['reprojection_partition_interface']='Artificial internal cut cap; neutral, not recovered source geometry'
    return obj,{'side':side,'nonmanifold_edges':bad,'degenerate_faces':deg,
                'volume':volume,'cap_faces':list(obj['reprojection_partition_cap_faces'])}


def _surface_proof(source, halves):
    old=source.data
    areas=[0.]*len(old.polygons)
    max_uv=max_plane=0.
    cap_area=0.
    for obj in halves:
        mesh=obj.data
        for face in mesh.polygons:
            index=mesh.attributes['partition_origin_face'].data[face.index].value
            if index<0:
                cap_area+=face.area
                continue
            original=old.polygons[index]
            if face.material_index!=original.material_index:
                raise AssertionError('Partition changed an original material assignment')
            areas[index]+=face.area
            loops=list(original.loop_indices)
            points=[old.vertices[old.loops[i].vertex_index].co for i in loops]
            a=points[0];bi=1
            ci=next(i for i in range(2,len(points)) if (points[bi]-a).cross(points[i]-a).length>1e-7)
            ab,ac=points[bi]-a,points[ci]-a
            aa,bb,cc=ab.dot(ab),ab.dot(ac),ac.dot(ac)
            det=aa*cc-bb*bb
            for li in face.loop_indices:
                point=mesh.vertices[mesh.loops[li].vertex_index].co
                ap=point-a
                u=(cc*ap.dot(ab)-bb*ap.dot(ac))/det
                v=(aa*ap.dot(ac)-bb*ap.dot(ab))/det
                max_plane=max(max_plane,abs(ap.dot(original.normal)))
                for layer in old.uv_layers:
                    uv=[layer.data[loops[k]].uv for k in (0,bi,ci)]
                    expected=uv[0]*(1-u-v)+uv[1]*u+uv[2]*v
                    actual=mesh.uv_layers[layer.name].data[li].uv
                    max_uv=max(max_uv,(expected-actual).length)
    area_error=max(abs(areas[p.index]-p.area) for p in old.polygons)
    relative=max(abs(areas[p.index]-p.area)/max(1.,p.area) for p in old.polygons)
    if relative>2e-5 or max_uv>2e-5 or max_plane>.001:
        raise AssertionError(f'Partition proof failed: area {relative}, UV {max_uv}, plane {max_plane}')
    return {'original_face_count':len(old.polygons),'max_face_area_error':area_error,
            'max_relative_face_area_error':relative,'max_uv_interpolation_error':max_uv,
            'max_original_plane_error':max_plane,'artificial_cap_total_area':cap_area,
            'original_surface_union_preserved':True,'caps_excluded_from_original_surface_union':True}


def partition():
    collection=bpy.data.collections['Derby Working']
    sources=[o for o in collection.objects if o.type=='MESH' and not o.hide_render and o.get('source_node')=='building-045']
    if len(sources)!=1 or sources[0].get('projection_component','').startswith('west-wall-walk-'):
        raise ValueError('Open the immutable pre-partition grouped model')
    source=sources[0]
    approved=[o for o in collection.objects if o.get('asset_group') in
              ('derby-lower-west-access-stair','derby-lower-west-wall-turret')]
    frozen={o:(o.parent,o.matrix_world.copy(),o.data if o.type=='MESH' else None,o.name) for o in approved}
    # Exactly the walkway cross-section between authored footprint vertices13/25.
    a=Vector((331.58942,-1947.3008/math.sin(math.radians(35)),0))
    b=Vector((364.7015,-1970.3208/math.sin(math.radians(35)),0))
    direction=b-a
    normal=Vector((direction.y,-direction.x,0)).normalized()
    inverse=source.matrix_world.inverted()
    plane_co=inverse@a
    plane_no=(source.matrix_world.to_3x3().transposed()@normal).normalized()
    neutral=bpy.data.materials.new('West wall walk / neutral artificial cut interface')
    neutral.diffuse_color=(.25,.25,.25,1)
    neutral['projection_preserve']=True
    halves=[];reports=[]
    for side in SECTIONS:
        obj,report=_half(source,side,plane_co,plane_no,neutral)
        halves.append(obj);reports.append(report)
    proof=_surface_proof(source,halves)
    old_parent=source.parent
    parents={}
    for side,(identifier,name,numbers) in SECTIONS.items():
        root=bpy.data.objects.new(name,None);collection.objects.link(root)
        root.parent=old_parent.parent;root.matrix_world=old_parent.matrix_world.copy()
        root['asset_group']=identifier;root['asset_name']=name
        parents[side]=root
        owned=[o for o in collection.objects if o.type=='MESH' and o.get('source_node') in
               {f'building-{n:03d}' for n in numbers}]
        owned.append(halves[list(SECTIONS).index(side)])
        for obj in owned:
            matrix=obj.matrix_world.copy()
            obj.parent=root;obj.matrix_world=matrix
            obj['asset_group']=identifier;obj['asset_name']=name
            obj.name=name+' / '+obj.get('part_name',obj['source_node'])
    bpy.context.view_layer.update()
    for obj,values in frozen.items():
        if (obj.parent,obj.matrix_world,obj.data if obj.type=='MESH' else None,obj.name)!=values:
            raise AssertionError('Approved stair or turret was changed')
    bpy.data.objects.remove(source,do_unlink=True)
    if old_parent.children:
        raise AssertionError('Unexpected children remain in retired wall-walk group')
    bpy.data.objects.remove(old_parent,do_unlink=True)
    return {'sections':reports,'surface_proof':proof,'approved_stair_and_turret_unchanged':True,
            'canonical_source_node':'building-045','projection_components':['west-wall-walk-north','west-wall-walk-south'],
            'junction_world':[list(a),list(b)],'status':'candidate pending component-aware catalog routing and user approval'}
