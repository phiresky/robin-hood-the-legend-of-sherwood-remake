"""Audited upper west curtain battlements, access flight and roof thickness."""
from pathlib import Path
import runpy

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG='upper-west-curtain-v1'


def _source(working,node):
    matches=[o for o in working.objects if o.type=='MESH' and o.get('source_node')==node and not o.hide_render]
    if len(matches)!=1:raise RuntimeError('Expected one baseline for '+node)
    return matches[0]


def _stair(source,helpers,directory):
    points=[source.matrix_world@v.co for v in source.data.vertices]
    project=helpers['_project']
    face=source.data.polygons[7]
    inverse=Matrix([project(points[i]) for i in face.vertices]).transposed().inverted()
    coords=[source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices]
    mesh=bpy.data.meshes.new('Upper west access stair construction')
    mesh.from_pydata([points[i] for i in (13,14,15,16)],[],[(0,1,2),(0,2,3)])
    uv=mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    for loop in mesh.loops:
        weights=inverse@project(mesh.vertices[loop.vertex_index].co)
        uv.data[loop.index].uv=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
    for material in source.data.materials:mesh.materials.append(material)
    proxy=bpy.data.objects.new(source.name+' / access flight',mesh)
    proxy.parent=source.parent;proxy.matrix_world=Matrix.Identity(4)
    for key in source.keys():proxy[key]=source[key]
    add_steps=runpy.run_path(str(directory/'derby_stair_details.py'))['add_steps']
    result=add_steps(proxy,(0,1),13)
    obj=bpy.data.objects[result['object']]
    obj['upper_west_curtain_refinement']=TAG
    obj.data.uv_layers[0].name=source.data.uv_layers[0].name
    obj.data.attributes.new('reprojection_fallback_material','INT','FACE')
    bpy.data.objects.remove(proxy,do_unlink=True)
    bpy.data.meshes.remove(mesh)
    return result


def _roof(source,repair,project):
    top=[source.matrix_world@source.data.vertices[i].co for i in (8,9,10,11)]
    bottom=[p-Vector((0,0,3.8)) for p in top]
    mesh=bpy.data.meshes.new(source.name+' / closed roof slab')
    mesh.from_pydata(top+bottom,[],[(0,1,2,3),(7,6,5,4),
                                 (0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bad=sum(not e.is_manifold for e in bm.edges)
    degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
    bm.to_mesh(mesh);bm.free()
    if bad or degenerate:raise RuntimeError('Invalid covered wall roof slab')
    mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    for material in source.data.materials:mesh.materials.append(material)
    mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    obj=bpy.data.objects.new(source.name+' / closed roof slab',mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():obj[key]=source[key]
    donor=source.data.polygons[4]
    points=[source.matrix_world@v.co for v in source.data.vertices]
    inverse=Matrix([project(points[i]) for i in donor.vertices]).transposed().inverted()
    coords=[source.data.uv_layers[0].data[i].uv.copy() for i in donor.loop_indices]
    for loop in mesh.loops:
        point=mesh.vertices[loop.vertex_index].co.copy()
        if loop.vertex_index>=4:point.z+=3.8
        weights=inverse@project(point)
        mesh.uv_layers[0].data[loop.index].uv=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
    obj['upper_west_curtain_refinement']=TAG
    source['upper_west_curtain_baseline']=TAG
    source.hide_render=True;source.hide_set(True)
    return {'object':obj.name,'source_node':obj['source_node'],
            'nonmanifold_edges':bad,'degenerate_faces':degenerate,'roof_thickness':3.8}


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.objects if o.get('upper_west_curtain_refinement')==TAG]
    if existing:
        if len(existing)!=3:raise RuntimeError('Incomplete upper west curtain pass')
        return {'reused':True,'objects':[o.name for o in existing]}
    bpy.context.view_layer.update()
    directory=Path(__file__).resolve().parent
    helpers=runpy.run_path(str(directory/'derby_asset_lower_west_curtain.py'))
    repair=runpy.run_path(str(directory/'derby_asset_east_bailey_west_curtain.py'))['_repair_fallback']
    source=_source(working,'building-117')
    cuts=[
        (68,69,0,((179,185),(199,205),(219,225),(239,245),(259,265),
                  (279,285),(299,305),(319,325),(339,345),(359,365),
                  (379,385),(399,405))),
        (76,75,0,((184,190),(201,207),(218,224),(231,237))),
        (77,76,0,((146,158),)),
        (81,77,0,((111,122),)),
        (82,81,1,((1040,1049),)),
        (82,78,0,((101,110),)),
        (78,68,0,((132,147),)),
    ]
    wall=helpers['_wall'](source,cuts)
    obj=bpy.data.objects[wall['object']]
    original_uv=[loop.uv.copy() for loop in obj.data.uv_layers[0].data]
    repair(obj,source,helpers['_project'])
    top=max(v.co.z for v in obj.data.vertices)
    for face in obj.data.polygons:
        if min(obj.data.vertices[i].co.z for i in face.vertices)<=top-27:
            for li in face.loop_indices:obj.data.uv_layers[0].data[li].uv=original_uv[li]
    obj['upper_west_curtain_refinement']=TAG
    del obj['lower_west_curtain_refinement']
    source['upper_west_curtain_baseline']=TAG
    stair=_stair(_source(working,'building-114'),helpers,directory)
    roof=_roof(_source(working,'building-073'),repair,helpers['_project'])
    return {'wall':wall,'stair':stair,'roof':roof,
            'retained_parts':['building-074','building-116','building-127','building-128'],
            'remaining':'Arrow slit depth, tiny corbel relief, and concealed roof underside artwork remain uncertain.'}
