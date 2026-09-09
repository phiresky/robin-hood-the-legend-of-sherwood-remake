"""Create a separate animated inspection scene through Blender MCP.

Run once, then call render_frame(frame, mode) from this module in MCP batches.
The camera makes one revolution in 300 frames at 25 fps. Source atlas motion
is supplemented by subtle inferred wind on isolated presentation copies.
"""
import json
import math
from pathlib import Path
import bpy
from mathutils import Vector, Matrix

ORIGINAL = globals().get('ORIGINAL_BASELINE', False)
FAST = globals().get('FAST_PREVIEW', False) or ORIGINAL
OUT = Path(bpy.data.filepath).parent / ('turntable-original' if ORIGINAL else 'turntable-fast' if FAST else 'turntable')
OUT.mkdir(exist_ok=True)
NAME = 'Sherwood Original Turntable' if ORIGINAL else 'Sherwood Fast Turntable' if FAST else 'Sherwood Turntable'
FRAMES = 300
MODES = ('textured', 'solid', 'wireframe')
WIDTH, HEIGHT = (960, 640) if FAST else (1440, 1080)


def setup():
    if NAME in bpy.data.scenes:
        raise RuntimeError('Turntable already exists; reuse render_frame without setup')
    source = bpy.data.scenes['Sherwood Refinement']
    scene = bpy.data.scenes.new(NAME)
    bpy.context.window.scene = scene
    scene.render.resolution_x = WIDTH
    scene.render.resolution_y = HEIGHT
    scene.render.resolution_percentage = 100
    scene.render.fps = 25
    scene.frame_start = 1
    scene.frame_end = FRAMES
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGBA'
    scene.render.film_transparent = True
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    scene.display.shading.light = 'STUDIO'
    scene.display.shading.color_type = 'SINGLE'
    scene.display.shading.single_color = (.58, .63, .66)
    scene.display.shading.show_shadows = True
    scene.display.shading.show_cavity = True
    scene.display.shading.cavity_type = 'BOTH'
    scene.display.shading.show_specular_highlight = True
    if FAST:
        scene.eevee.taa_render_samples = 8
        scene.display.render_aa = 'FXAA'
    copies = []
    pivots = {}
    for collection in source.collection.children:
        if ORIGINAL:
            if not collection.name.startswith('00 '):
                continue
        elif collection.hide_render or collection.name.startswith(('00 ', '02 ', '07 ')):
            continue
        group = bpy.data.collections.new('TURNTABLE ' + collection.name)
        scene.collection.children.link(group)
        for original in collection.objects:
            if original.type != 'MESH' or (original.hide_render and not ORIGINAL):
                continue
            obj = original.copy()
            obj.name = 'TURNTABLE ' + original.name
            obj.parent = None
            obj.matrix_world = original.matrix_world.copy()
            obj.hide_render = False
            obj.hide_viewport = False
            group.objects.link(obj)
            copies.append(obj)
            if collection.name.startswith('10 '):
                key = (original['profile'], original['supporting_tree'])
                if key not in pivots:
                    branches = next(o for o in collection.objects if o['profile'] == key[0]
                                    and o['supporting_tree'] == key[1] and 'limbs forks' in o.name)
                    points = [branches.matrix_world @ v.co for v in branches.data.vertices]
                    low = min(p.z for p in points)
                    base = [p for p in points if p.z < low + 2]
                    pivot = bpy.data.objects.new('Wind pivot ' + str(key), None)
                    pivot.location = sum(base, Vector()) / len(base)
                    group.objects.link(pivot)
                    phase = len(pivots) * 1.73
                    for axis, amplitude in [(0, .0045), (1, .003)]:
                        d = pivot.driver_add('rotation_euler', axis).driver
                        d.expression = f'{amplitude}*(sin((frame-1)*2*pi/75+{phase})-sin({phase}))'
                    pivots[key] = pivot
                world = obj.matrix_world.copy()
                obj.parent = pivots[key]
                obj.matrix_parent_inverse = Matrix.Translation(-pivots[key].location)
                obj.matrix_basis = world
            if collection.name.startswith('11 '):
                obj['turntable_ambient'] = True
    if not ORIGINAL:
        animate_ambient(scene)
    bpy.context.view_layer.update()
    points = [o.matrix_world @ Vector(p) for o in copies for p in o.bound_box]
    lo = Vector([min(p[i] for p in points) for i in range(3)])
    hi = Vector([max(p[i] for p in points) for i in range(3)])
    center = (lo + hi) / 2
    center.z -= 300 if FAST else 150
    camera = bpy.data.objects.new('Turntable camera', bpy.data.cameras.new('Turntable orthographic'))
    scene.collection.objects.link(camera)
    scene.camera = camera
    camera.data.type = 'ORTHO'
    camera.data.clip_end = 15000
    radius = 6000
    elevation = math.radians(38)
    extent = 0
    for frame in range(1, FRAMES + 2):
        angle = (frame - 1) * 2 * math.pi / FRAMES
        direction = Vector((math.sin(angle) * math.cos(elevation),
                            -math.cos(angle) * math.cos(elevation), math.sin(elevation)))
        camera.location = center + radius * direction
        camera.rotation_euler = (-direction).to_track_quat('-Z', 'Y').to_euler()
        camera.keyframe_insert('location', frame=frame)
        camera.keyframe_insert('rotation_euler', frame=frame)
        rotation = camera.rotation_euler.to_matrix().transposed()
        for p in points:
            q = rotation @ (p - center)
            extent = max(extent, abs(q.x) * 2, abs(q.y) * 2 * WIDTH / HEIGHT)
    camera.data.ortho_scale = 2350 if FAST else extent * 1.09
    if ORIGINAL:
        # Exact camera/action copy: the center split compares coincident pixels.
        old_camera = camera
        after_camera = bpy.data.scenes['Sherwood Fast Turntable'].camera
        camera = after_camera.copy()
        camera.data = after_camera.data.copy()
        camera.name = 'Original baseline - matched turntable camera'
        scene.collection.objects.link(camera)
        scene.camera = camera
        bpy.data.objects.remove(old_camera, do_unlink=True)
    scene['animation_notes'] = 'Authored tree and ambient atlas frames at 25 Hz; inferred branch sway for inspection copies.'
    scene['source_limit'] = 'Single-view reconstruction; rear geometry and colors inferred.'
    scene['turntable_scale'] = camera.data.ortho_scale
    scene.frame_set(1)
    scene.render.engine = 'BLENDER_EEVEE'
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    return {'objects': len(copies), 'wind_pivots': len(pivots), 'scale': camera.data.ortho_scale}


def animate_ambient(scene):
    root = OUT.parent / 'animation-references'
    records = json.loads((root / 'turntable-fx.json').read_text())
    for obj in [o for o in scene.objects if o.get('turntable_ambient')]:
        # Names on the generated ambient objects retain the source profile.
        r = next(r for r in records if r['profile'] in obj.name)
        obj.data = obj.data.copy()
        w, h = r['canvas']
        x = r['position'][0] + r['canvas_offset'][0]
        y = r['position'][1] + r['canvas_offset'][1]
        sin = math.sin(math.radians(35))
        # Existing overlay geometry uses world-space ground projection.
        positions = [(x, -y / sin, .15), (x+w, -y/sin, .15),
                     (x+w, -(y+h)/sin, .15), (x, -(y+h)/sin, .15)]
        if len(obj.data.vertices) != 4:
            raise RuntimeError('Expected four-corner ambient reference: ' + obj.name)
        for vertex, position in zip(obj.data.vertices, positions):
            vertex.co = obj.matrix_world.inverted() @ Vector(position)
        mat = obj.data.materials[0].copy()
        obj.data.materials.clear()
        obj.data.materials.append(mat)
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        texture = next(n for n in nodes if n.type == 'TEX_IMAGE')
        texture.image = bpy.data.images.load(str(root / r['turntable_atlas']), check_existing=True)
        texture.image.pack()
        uv = nodes.new('ShaderNodeTexCoord')
        scale = nodes.new('ShaderNodeVectorMath'); scale.operation = 'MULTIPLY'
        aw, ah = r['atlas_size']
        scale.inputs[1].default_value = (w/aw, h/ah, 1)
        offset = nodes.new('ShaderNodeVectorMath'); offset.operation = 'ADD'
        links.new(uv.outputs['UV'], scale.inputs[0]); links.new(scale.outputs[0], offset.inputs[0])
        links.new(offset.outputs[0], texture.inputs['Vector'])
        durations = [d + 1 for d in r['delays']]
        if len(set(durations)) != 1:
            raise RuntimeError('Variable frame delays require a keyed atlas timeline')
        index = f'(floor((frame-1)/{durations[0]})%{len(durations)})'
        cols, rows = r['columns'], r['rows']
        for axis, expression in enumerate([f'((({index})%{cols})*{w+4}+2)/{aw}',
                                           f'(({rows-1}-floor(({index})/{cols}))*{h+4}+2)/{ah}']):
            driver = offset.inputs[1].driver_add('default_value', axis).driver
            driver.expression = expression


def modes_for_frame(frame):
    # Hold each mode, then reveal the next with a 20-frame swipe. End textured.
    for start, old, new in [(81, 'textured', 'solid'), (181, 'solid', 'wireframe'),
                            (281, 'wireframe', 'textured')]:
        if start <= frame <= start + 19:
            return [old, new]
    return ['textured' if frame < 81 else 'solid' if frame < 181 else 'wireframe']


def render_frames():
    # Keep authored motion at its original speed while sampling fewer frames.
    return [1 + round(i * 25 / 15) for i in range(180)] if FAST else list(range(1, 301))


def render_frame(frame, mode):
    scene = bpy.data.scenes[NAME]
    bpy.context.window.scene = scene
    scene.frame_set(frame)
    scene.render.engine = 'BLENDER_WORKBENCH' if mode == 'solid' else 'BLENDER_EEVEE'
    wire = next(m for m in bpy.data.materials if m.name.startswith('INSPECTION - dark topology lines'))
    scene.view_layers[0].material_override = wire if mode == 'wireframe' else None
    for obj in scene.objects:
        if obj.get('turntable_ambient'):
            obj.hide_render = mode != 'textured'
    path = OUT / f'{mode}-{frame:04}.png'
    if FAST:
        # Animation batches keep the render engine alive across frames. A still
        # render per frame repeatedly pays scene/engine initialization costs.
        old_range = (scene.frame_start, scene.frame_end, scene.frame_step)
        end = 100 if mode == 'textured' and frame < 101 else 200 if mode == 'solid' else 300
        scene.frame_start = frame
        scene.frame_end = min(frame + 14, end)
        scene.frame_step = 1
        scene.render.filepath = str(OUT / f'{mode}-')
        try:
            bpy.ops.render.render(animation=True)
        finally:
            scene.frame_start, scene.frame_end, scene.frame_step = old_range
        if not path.is_file():
            raise RuntimeError('Animation render did not produce ' + str(path))
    else:
        scene.render.filepath = str(path)
        bpy.ops.render.render(write_still=True)
    return str(path)


if __name__ == '__main__':
    result = setup()
