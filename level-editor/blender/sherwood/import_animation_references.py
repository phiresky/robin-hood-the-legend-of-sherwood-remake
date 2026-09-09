"""Import separate authored sprite references through Blender MCP.

These are explicitly reference cards, not reconstructed 3D trees. Tree atlases
retain all 16 frames and frame offsets, with the default engine delay+1 timing.
The animation inspection scene has its own timeline, leaving camera bookmarks
in the geometry inspection scene intact.
"""

import json
import math
from pathlib import Path

import bpy
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/sherwood-refinement/animation-references'
SIN=math.sin(math.radians(35))
COS=math.cos(math.radians(35))
NAME='07 Reference only - authored animated sprites'
if NAME in bpy.data.collections:
    raise RuntimeError('Animation reference collection already exists')
records=json.loads((OUT/'manifest.json').read_text())['assets']
main=bpy.data.scenes['Sherwood Refinement']
collection=bpy.data.collections.new(NAME)
main.collection.children.link(collection)


def material_for(record):
    material=bpy.data.materials.new('REFERENCE '+record['profile'])
    material.use_nodes=True
    nodes=material.node_tree.nodes
    nodes.clear()
    uv=nodes.new('ShaderNodeTexCoord')
    texture=nodes.new('ShaderNodeTexImage')
    is_tree=record['kind']=='tree'
    texture.image=bpy.data.images.load(str(OUT/record['atlas' if is_tree else 'first_png']),check_existing=True)
    texture.image.pack()
    texture.extension='CLIP'
    texture.interpolation='Closest'
    links=material.node_tree.links
    if is_tree:
        width,height=record['canvas']
        aw,ah=record['atlas_size']
        gutter=record['gutter']
        multiply=nodes.new('ShaderNodeVectorMath');multiply.operation='MULTIPLY'
        multiply.inputs[1].default_value=(width/aw,height/ah,1)
        add=nodes.new('ShaderNodeVectorMath');add.operation='ADD'
        links.new(uv.outputs['UV'],multiply.inputs[0])
        links.new(multiply.outputs[0],add.inputs[0])
        index='(floor((frame-1)/4)%16)'
        expressions=[f'(({index}%4)*{width+2*gutter}+{gutter})/{aw}',
                     f'((3-floor({index}/4))*{height+2*gutter}+{gutter})/{ah}']
        for axis,expression in enumerate(expressions):
            driver=add.inputs[1].driver_add('default_value',axis).driver
            driver.type='SCRIPTED';driver.expression=expression
        links.new(add.outputs[0],texture.inputs['Vector'])
    else:
        links.new(uv.outputs['UV'],texture.inputs['Vector'])
    emission=nodes.new('ShaderNodeEmission')
    transparent=nodes.new('ShaderNodeBsdfTransparent')
    mix=nodes.new('ShaderNodeMixShader')
    output=nodes.new('ShaderNodeOutputMaterial')
    links.new(texture.outputs['Color'],emission.inputs['Color'])
    links.new(texture.outputs['Alpha'],mix.inputs[0])
    links.new(transparent.outputs[0],mix.inputs[1])
    links.new(emission.outputs[0],mix.inputs[2])
    links.new(mix.outputs[0],output.inputs['Surface'])
    return material


for record in records:
    material=material_for(record)
    if record['kind']=='tree':
        width,height=record['canvas']
        ox,oy=record['canvas_offset']
        x,y=record['position']
        cx,cy=record['center']
        elevation=record['elevation']
        anchor=Vector((x+cx,-(y+cy+elevation)/SIN,elevation/COS))
        down=Vector((0,-SIN,-COS))
        top_left=anchor+Vector((ox-cx,0,0))+down*(oy-cy)
    else:
        # Ground-projected reference overlays for the other FX. They retain
        # screen placement, not a claim about physical flame/water geometry.
        width,height=record['size']
        top_left=Vector((record['left'],-record['top']/SIN-0.05,0.04))
        down=Vector((0,-1/SIN,0))
    right=Vector((width,0,0))
    vertices=[top_left,top_left+right,top_left+right+down*height,top_left+down*height]
    mesh=bpy.data.meshes.new('REFERENCE '+record['profile'])
    mesh.from_pydata(vertices,[],[(0,3,2,1)])
    mesh.update()
    obj=bpy.data.objects.new(mesh.name,mesh)
    collection.objects.link(obj)
    mesh.materials.append(material)
    uv=mesh.uv_layers.new(name='Sprite canvas')
    coords=[(0,1),(1,1),(1,0),(0,0)]
    for loop in mesh.loops:uv.data[loop.index].uv=coords[loop.vertex_index]
    obj['reference_only']=True
    obj['not_reconstructed_geometry']='Authored animated sprite card for tracing and comparison'
    for key in ('profile','bank','elevation','frame_count','force_display','blit_type'):
        obj[key]=record[key]
    obj['source_position']=record['position']
    obj['source_delays']=record['delays']
    obj['source_frame_offsets']=json.dumps(record['offsets'])

# Scene-specific exclusion keeps flat cards out of geometry/solid inspections.
bpy.context.window.scene=main
bpy.context.view_layer.update()
main.view_layers[0].layer_collection.children[NAME].exclude=True
scene=bpy.data.scenes.new('Sherwood Animation Reference')
for c in main.collection.children:
    if not c.name.startswith('00 '):scene.collection.children.link(c)
camera=bpy.data.objects['Reference Camera']
if camera.name not in scene.objects:scene.collection.objects.link(camera)
scene.camera=camera
scene.render.engine='BLENDER_EEVEE'
scene.render.resolution_x=1920;scene.render.resolution_y=1088;scene.render.resolution_percentage=100
scene.render.fps=25;scene.frame_start=1;scene.frame_end=64
scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
scene.world=main.world
scene['reference_limit']='Synchronized authored tree frames; first-frame ambient FX; not runtime phase or full rendering parity'
scene['timing_source']='sprite.rs increment_frame: advances when frame_count > delay; mission runtime 25 Hz'
reference=bpy.data.images.load(str(OUT/'sherwood-composite-frame00.png'),check_existing=True)
reference.pack()
background=camera.data.background_images.new()
background.image=reference;background.alpha=0.5;background.display_depth='FRONT';background.show_background_image=False
bpy.context.window.scene=scene
scene.frame_set(1)
bpy.ops.file.pack_all()
bpy.ops.wm.save_as_mainfile(filepath=str(OUT.parent/'sherwood-refinement.blend'))
result={'scene':scene.name,'reference_cards':len(records),'animated_trees':sum(r['kind']=='tree' for r in records),'frames':64,'fps':25}
