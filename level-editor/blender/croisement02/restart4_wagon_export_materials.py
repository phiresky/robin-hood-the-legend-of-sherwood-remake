"""Flatten native/fallback color mixing only in a temporary wagon export scene."""
import bpy

def flatten(objects):
    scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=1
    records=[]
    for obj in objects:
        used={p.material_index for p in obj.data.polygons}
        composite={i for i in used if any(n.type=='MIX_RGB' for n in obj.data.materials[i].node_tree.nodes)}
        if not composite:continue
        atlas_mat=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'))
        atlas_node=next(n for n in atlas_mat.node_tree.nodes if n.type=='TEX_IMAGE')
        uv_name=atlas_node.inputs['Vector'].links[0].from_node.uv_map
        target=bpy.data.images.new('Export combined '+obj.name,width=atlas_node.image.size[0],height=atlas_node.image.size[1],alpha=False)
        target.colorspace_settings.name='sRGB'
        original=list(obj.data.materials);temporary=[]
        for i,mat in enumerate(original):
            if i not in used:continue
            copied=mat.copy();obj.data.materials[i]=copied;nodes=copied.node_tree.nodes;links=copied.node_tree.links
            output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL');shader=output.inputs['Surface'].links[0].from_node
            incoming=output.inputs['Surface'].links[0].from_socket
            emission=nodes.new('ShaderNodeEmission')
            if incoming.type=='RGBA':links.new(incoming,emission.inputs['Color'])
            else:
                color=shader.inputs['Base Color'] if shader.type=='BSDF_PRINCIPLED' else shader.inputs['Color']
                if color.is_linked:links.new(color.links[0].from_socket,emission.inputs['Color'])
                else:emission.inputs['Color'].default_value=color.default_value
            links.new(emission.outputs[0],output.inputs['Surface']);node=nodes.new('ShaderNodeTexImage');node.image=target
            for n in nodes:n.select=False
            node.select=True;nodes.active=node;temporary.append(copied)
        obj.data.uv_layers.active=obj.data.uv_layers[uv_name]
        bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
        bpy.ops.object.bake(type='EMIT',use_clear=True,margin=2)
        target.pack()
        for i,mat in enumerate(original):obj.data.materials[i]=mat
        for i in composite:
            old=original[i];oldshader=next(n for n in old.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
            mat=bpy.data.materials.new(old.name+' / flattened export color');mat.use_nodes=True
            shader=mat.node_tree.nodes.get('Principled BSDF');shader.inputs['Roughness'].default_value=oldshader.inputs['Roughness'].default_value;shader.inputs['Emission Strength'].default_value=oldshader.inputs['Emission Strength'].default_value
            tex=mat.node_tree.nodes.new('ShaderNodeTexImage');tex.image=target;tex.interpolation='Closest';uv=mat.node_tree.nodes.new('ShaderNodeUVMap');uv.uv_map=uv_name
            mat.node_tree.links.new(uv.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Color'],shader.inputs['Base Color']);mat.node_tree.links.new(tex.outputs['Color'],shader.inputs['Emission Color']);obj.data.materials[i]=mat
        records.append(dict(object=obj.name,flattened_material_indices=sorted(composite),atlas_size=list(target.size),uv_map=uv_name,method='EMIT bake of combined color only into existing generated atlas UV; original per-face shader lighting retained'))
    bpy.ops.object.select_all(action='DESELECT')
    for obj in objects:obj.select_set(True)
    return records
