"""Native foliage material and mesh construction for Croisement03."""
import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
RAY=Vector((0,-COS,SIN))

def material(name,path,known=True):
    mat=bpy.data.materials.new(name);mat.use_nodes=True
    mat.use_backface_culling=False
    if hasattr(mat,'surface_render_method'):mat.surface_render_method='DITHERED'
    mat['foliage_physical_opacity']=True;mat['foliage_alpha_cutoff']=.5
    mat['opacity_semantics']='physical-coverage'
    mat['projection_preserve']=True;mat['source_ownership_semantics']='separate-mask'
    mat['source_ownership_channel']='vertex-color-r';mat['foliage_observed']=known
    mat['texture_provenance']='observed front' if known else 'inferred rear/side using same Croisement03 foliage'
    nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Foliage UV'
    tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path),check_existing=False);tex.image.pack();tex.interpolation='Closest'
    shader=nodes.new('ShaderNodeBsdfPrincipled');shader.inputs['Roughness'].default_value=1
    output=nodes.new('ShaderNodeOutputMaterial');links.new(uv.outputs['UV'],tex.inputs['Vector'])
    links.new(tex.outputs['Color'],shader.inputs['Base Color']);links.new(tex.outputs['Alpha'],shader.inputs['Alpha'])
    links.new(tex.outputs['Color'],shader.inputs['Emission Color']);shader.inputs['Emission Strength'].default_value=1
    links.new(shader.outputs[0],output.inputs[0]);return mat


def one_sided(mat):
    """Match explicit Cycles culling to the reference models' raster flag."""
    mat.use_backface_culling=True;mat['foliage_card_sides']='paired-one-sided'
    nodes=mat.node_tree.nodes;links=mat.node_tree.links
    if nodes.get('One-sided foliage'):return
    output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL')
    original=output.inputs['Surface'].links[0].from_socket
    geometry=nodes.new('ShaderNodeNewGeometry');transparent=nodes.new('ShaderNodeBsdfTransparent')
    mix=nodes.new('ShaderNodeMixShader');mix.name='One-sided foliage'
    links.new(geometry.outputs['Backfacing'],mix.inputs[0]);links.new(original,mix.inputs[1]);links.new(transparent.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],output.inputs['Surface'])


def replace_mesh(obj,vertices,faces,uvs=None,materials=(),slots=None,known=None):
    mesh=bpy.data.meshes.new(obj.name+' refined');inverse=obj.matrix_world.inverted()
    mesh.from_pydata([inverse@Vector(p) for p in vertices],[],faces);mesh.update()
    for mat in materials:mesh.materials.append(mat)
    uv=mesh.uv_layers.new(name='Foliage UV')
    ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
    mesh.color_attributes.active_color=ownership
    for face in mesh.polygons:
        if slots is not None:face.material_index=slots[face.index]
        for loop in face.loop_indices:
            vertex=mesh.loops[loop].vertex_index
            uv.data[loop].uv=uvs[vertex] if uvs else (vertices[vertex][0]/1408,1-(-vertices[vertex][1]*SIN-vertices[vertex][2]*COS)/960)
            ownership.data[loop].color=(float(known[face.index]) if known else 0.,1,1,1)
    bm=bmesh.new();bm.from_mesh(mesh)
    degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
    boundary=sum(e.is_boundary for e in bm.edges);nonmanifold=sum(not e.is_manifold for e in bm.edges)
    if degenerate:raise ValueError(f'{obj.name}: {degenerate} degenerate faces')
    if not uvs:bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bm.to_mesh(mesh);bm.free();mesh.update();obj.data=mesh
    return dict(vertices=len(vertices),faces=len(faces),boundary_edges=boundary,nonmanifold_edges=nonmanifold,degenerate_faces=degenerate)

