"""Unfiltered object-index output for sign visibility, independent of RGB."""
import bpy


def setup(scene, bodies, neighbors):
    for obj in bodies:
        obj.pass_index = 777
    for obj in neighbors:
        obj.pass_index = 0
    for layer in scene.view_layers:
        layer.use_pass_object_index = True
        layer.pass_alpha_threshold = .5
        layer.update_render_passes()
    tree = bpy.data.node_groups.new('Sign body first-hit object index', 'CompositorNodeTree')
    tree.interface.new_socket(name='Image', in_out='OUTPUT', socket_type='NodeSocketColor')
    output = tree.nodes.new('NodeGroupOutput')
    rendered = tree.nodes.new('CompositorNodeRLayers')
    rendered.scene = scene
    rendered.layer = scene.view_layers[0].name
    mask = tree.nodes.new('CompositorNodeIDMask')
    mask.inputs['Index'].default_value = 777
    mask.inputs['Anti-Alias'].default_value = False
    tree.links.new(rendered.outputs['Object Index'], mask.inputs['ID value'])
    tree.links.new(mask.outputs['Alpha'], output.inputs['Image'])
    scene.compositing_node_group = tree
    scene.render.use_compositing = True
