"""Create and render the slower 60 fps turntable through Blender MCP.

Load with __name__='hq', then call setup() once and render_batch(side, mode,
start, count). Linked scene geometry preserves the original/refined comparison.
Native time remapping evaluates camera, atlas drivers and wind at subframes;
this does not duplicate or interpolate already-rendered video frames.
"""
import json
from pathlib import Path
import bpy

OUT = Path(bpy.data.filepath).parent/'turntable-hq'
NAMES = {'refined':'Sherwood HQ Turntable', 'original':'Sherwood HQ Original'}
FRAMES = range(5,1445)


def modes(frame):
    time = frame*5/24
    result = []
    if time < 101 or time >= 281:
        result.append('textured')
    if 81 <= time < 201:
        result.append('solid')
    if 181 <= time < 301:
        result.append('wireframe')
    return result


def setup():
    if any(name in bpy.data.scenes for name in NAMES.values()):
        raise RuntimeError('HQ scenes already exist; reuse render_batch')
    OUT.mkdir(exist_ok=True)
    (OUT/'original').mkdir(exist_ok=True)
    source = bpy.data.scenes['Sherwood Fast Turntable']
    bpy.context.window.scene = source
    poses = []
    previous = None
    for frame in range(1,302):
        source.frame_set(frame)
        rotation = source.camera.rotation_euler.copy()
        if previous is not None:
            rotation.make_compatible(previous)
        poses.append((source.camera.location.copy(), rotation))
        previous = rotation.copy()
    for side, name in NAMES.items():
        original = bpy.data.scenes['Sherwood Original Turntable' if side=='original' else 'Sherwood Fast Turntable']
        scene = original.copy()
        scene.name = name
        camera = original.camera.copy()
        camera.data = original.camera.data.copy()
        camera.animation_data_clear()
        scene.collection.objects.link(camera)
        scene.camera = camera
        for frame, (location, rotation) in enumerate(poses,1):
            camera.location = location
            camera.rotation_euler = rotation
            camera.keyframe_insert('location',frame=frame)
            camera.keyframe_insert('rotation_euler',frame=frame)
        scene.render.resolution_x = 1920
        scene.render.resolution_y = 1280
        scene.render.resolution_percentage = 100
        scene.render.fps = 60
        scene.render.frame_map_old = 5
        scene.render.frame_map_new = 24
        scene.render.image_settings.compression = 15
        scene.eevee.taa_render_samples = 16
        scene.display.render_aa = '16'
        scene.frame_start, scene.frame_end = 5,1444
        scene.frame_step = 1
        scene['animation_notes'] = '24 seconds / 60 fps; native 25 Hz timeline slowed 2x, evaluated at fractional frames. Camera Euler winding unwrapped.'
    # At fractional frames a wrapped Euler key would spin the camera near 180°.
    scene = bpy.data.scenes[NAMES['refined']]
    bpy.context.window.scene = scene
    angles = []
    for frame in range(715,735):
        scene.frame_set(frame)
        angles.append(scene.camera.rotation_euler.z)
    if max(abs(a-b) for a,b in zip(angles,angles[1:])) > .02:
        raise RuntimeError('HQ camera has a rotation discontinuity')
    for side in NAMES:
        bpy.data.scenes[NAMES[side]].frame_set(5)
    source.frame_set(1)
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    return {'scenes':NAMES,'frames':len(FRAMES),'fps':60,'seconds':24,'resolution':[1920,1280]}


def render_batch(side, mode, start, count=24):
    scene = bpy.data.scenes[NAMES[side]]
    bpy.context.window.scene = scene
    directory = OUT/'original' if side=='original' else OUT
    frames = list(range(start,min(start+count,1445)))
    frames = [f for f in frames if mode in modes(f)]
    if not frames or frames!=list(range(frames[0],frames[-1]+1)):
        raise ValueError('Batch must contain a consecutive valid mode interval')
    scene.render.engine = 'BLENDER_WORKBENCH' if mode=='solid' else 'BLENDER_EEVEE'
    # The other half is discarded by the fixed split; retain full-size RGBA
    # output coordinates while avoiding shading pixels that cannot be shown.
    scene.render.use_border = True
    scene.render.use_crop_to_border = False
    scene.render.border_min_x = .5 if side=='refined' else 0.
    scene.render.border_max_x = 1. if side=='refined' else .5
    scene.render.border_min_y, scene.render.border_max_y = 0.,1.
    wire = next(m for m in bpy.data.materials if m.name.startswith('INSPECTION - dark topology lines'))
    scene.view_layers[0].material_override = wire if mode=='wireframe' else None
    for obj in scene.objects:
        if obj.get('turntable_ambient'):
            obj.hide_render = mode!='textured'
    scene.frame_start,scene.frame_end = frames[0],frames[-1]
    scene.render.filepath = str(directory/f'{mode}-')
    bpy.ops.render.render(animation=True)
    missing = [f for f in frames if not (directory/f'{mode}-{f:04}.png').is_file()]
    if missing:
        raise RuntimeError(f'Missing HQ renders: {missing}')
    return {'side':side,'mode':mode,'first':frames[0],'last':frames[-1]}


def finish():
    count = 0
    for side in NAMES:
        directory = OUT/'original' if side=='original' else OUT
        for frame in FRAMES:
            for mode in modes(frame):
                if not (directory/f'{mode}-{frame:04}.png').is_file():
                    raise FileNotFoundError(directory/f'{mode}-{frame:04}.png')
                count += 1
    after = bpy.data.scenes[NAMES['refined']]
    before = bpy.data.scenes[NAMES['original']]
    camera_error = 0.
    for frame in [5,389,724,725,869,1349,1444]:
        matrices = []
        for scene in [before,after]:
            bpy.context.window.scene = scene
            scene.frame_set(frame)
            bpy.context.view_layer.update()
            matrices.append(scene.camera.matrix_world.copy())
        camera_error = max(camera_error,max(abs(matrices[0][r][c]-matrices[1][r][c])
                                            for r in range(4) for c in range(4)))
    if camera_error > 1e-5:
        raise RuntimeError(f'Comparison cameras do not match: {camera_error}')
    wind = next(o for o in after.objects if o.name.startswith('Wind pivot '))
    wind_values = []
    bpy.context.window.scene = after
    for frame in [5,29]:
        after.frame_set(frame)
        wind_values.append(tuple(wind.rotation_euler))
    if wind_values[0] == wind_values[1]:
        raise RuntimeError('HQ wind animation is not advancing')
    for scene in [before,after]:
        scene.frame_start,scene.frame_end = 5,1444
        scene.render.engine = 'BLENDER_EEVEE'
        scene.view_layers[0].material_override = None
        for obj in scene.objects:
            if obj.get('turntable_ambient'):
                obj.hide_render = False
        scene.frame_set(5)
    preview = bpy.data.scenes['Sherwood Fast Turntable']
    bpy.context.window.scene = preview
    preview.render.engine = 'BLENDER_EEVEE'
    preview.view_layers[0].material_override = None
    preview.frame_set(1)
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    report = {'rendered_pass_frames':count,'camera_matrix_error':camera_error,
              'wind_rotations':wind_values,'native_scene_saved':bpy.data.filepath}
    (OUT/'scene-validation.json').write_text(json.dumps(report,indent=2))
    return report


if __name__=='__main__':
    result=setup()
