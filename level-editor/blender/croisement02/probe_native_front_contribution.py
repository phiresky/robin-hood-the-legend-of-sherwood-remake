"""Read-only native-camera first-hit contribution of projected interior foliage."""
import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
for folder in ('level-editor/blender/croisement02', 'level-editor/refinement', 'level-editor/refinement/blender'):
    sys.path.insert(0, str(ROOT / folder))
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, RAY


def diagnostic_material(material, color):
    """Change emitted RGB only, retaining the existing physical-alpha graph."""
    result = material.copy()
    nodes, links = result.node_tree.nodes, result.node_tree.links
    shaders = 0
    for node in nodes:
        if node.type == 'BSDF_PRINCIPLED':
            for name, value in [('Base Color', (0, 0, 0, 1)), ('Emission Color', (*color, 1)),
                                ('Emission Strength', 1.), ('Specular IOR Level', 0.)]:
                socket = node.inputs.get(name)
                if socket is None:
                    raise ValueError(f'Missing diagnostic shader socket: {name}')
                for link in list(socket.links):
                    links.remove(link)
                socket.default_value = value
            shaders += 1
        elif node.type == 'EMISSION':
            for link in list(node.inputs['Color'].links):
                links.remove(link)
            node.inputs['Color'].default_value = (*color, 1)
            node.inputs['Strength'].default_value = 1
            shaders += 1
    if not shaders:
        outputs = [n for n in nodes if n.type == 'OUTPUT_MATERIAL']
        if len(outputs) != 1 or len(outputs[0].inputs['Surface'].links) != 1:
            raise ValueError(f'Unsupported physical material: {material.name}')
        source = outputs[0].inputs['Surface'].links[0].from_socket
        if source.type != 'RGBA':
            raise ValueError(f'Unsupported physical material: {material.name}')
        # A direct color-to-surface link is Blender's opaque implicit emission.
        emission = nodes.new('ShaderNodeEmission')
        emission.inputs['Color'].default_value = (*color, 1)
        emission.inputs['Strength'].default_value = 1
        links.new(emission.outputs[0], outputs[0].inputs['Surface'])
    return result


def main(worker, destination, scale):
    if destination.exists() or not 1 <= scale <= 8:
        raise ValueError('Use a fresh destination and scale1..8')
    model_hash = sha(worker / 'model.blend')
    cfg = json.loads((worker / 'workspace.json').read_text())
    coverage_path = worker / 'inspection/source-coverage/report.json'
    coverage = json.loads(coverage_path.read_text())
    if coverage['model_sha256'] != model_hash:
        raise ValueError('Source camera coverage belongs to another worker')
    left, top, right, bottom = coverage['source_crop']
    destination.mkdir(parents=True)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
        objects = [o for o in list(bpy.data.collections[cfg['collection_name']].all_objects)
                   if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id']]
        scene = bpy.data.scenes.new('Native first-hit contribution')
        counts = {}
        for original in objects:
            obj = original.copy()
            obj.data = original.data.copy()
            obj.parent = None
            obj.matrix_world = original.matrix_world.copy()
            obj.hide_render = False
            scene.collection.objects.link(obj)
            crown = obj.get('projection_component') == 'crown'
            used_slots = {p.material_index for p in obj.data.polygons}
            for slot, material in enumerate(list(obj.data.materials)):
                if slot not in used_slots:
                    continue
                group = 'projected_interior' if crown and slot == 5 else 'other_crown' if crown else 'wood'
                color = {'projected_interior': (1, 0, 0), 'other_crown': (0, 1, 0), 'wood': (0, 0, 1)}[group]
                obj.data.materials[slot] = diagnostic_material(material, color)
                counts[group] = counts.get(group, 0) + sum(p.material_index == slot for p in obj.data.polygons)
        width, height = right - left, bottom - top
        target = Vector(((left + right) / 2, -(top + bottom) / 2 / SIN, 0))
        data = bpy.data.cameras.new('Exact native contribution camera')
        data.type = 'ORTHO'
        data.sensor_fit = 'HORIZONTAL'
        data.ortho_scale = width
        data.clip_end = 20000
        camera = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(camera)
        camera.location = target + RAY * 5000
        camera.rotation_euler = (target - camera.location).to_track_quat('-Z', 'Y').to_euler()
        scene.camera = camera
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.seed = 0
        scene.cycles.transparent_max_bounces = 256
        scene.render.resolution_x, scene.render.resolution_y = width * scale, height * scale
        scene.render.resolution_percentage = 100
        scene.render.film_transparent = True
        scene.render.image_settings.file_format = 'PNG'
        scene.render.image_settings.color_mode = 'RGBA'
        scene.view_settings.view_transform = 'Standard'
        scene.view_settings.look = 'None'
        scene.render.filepath = str(destination / 'native-groups.png')
        bpy.ops.render.render(write_still=True, scene=scene.name)
        rgba = np.asarray(Image.open(destination / 'native-groups.png').convert('RGBA'))
        visible = rgba[..., 3] > 127
        owner = np.argmax(rgba[..., :3], axis=2)
        projected = visible & (owner == 0) & (rgba[..., 0] > 127)
        Image.fromarray(projected.astype('uint8') * 255).save(destination / 'projected-interior-first-hit.png')
        if sha(worker / 'model.blend') != model_hash:
            raise RuntimeError('Diagnostic changed its source model')
        write_json(destination / 'evidence.json', dict(worker=str(worker), model_sha256=model_hash,
            source_camera_report_sha256=sha(coverage_path), native_crop=coverage['source_crop'], scale=scale,
            material_groups=counts, native_first_hit_samples=int(visible.sum()),
            projected_interior_first_hit_samples=int(projected.sum()),
            projected_interior_fraction=float(projected.sum() / visible.sum()) if visible.any() else None,
            render_config=dict(samples=8, transparent_max_bounces=256, engine='CYCLES'),
            worker_unchanged=True, approval='diagnostic only; does not authorize changing ownership',
            limitation='Group-level antialiased first-hit estimate. A conservative per-texel visibility proof is required before changing protected atlas ownership.',
            file_sha256={p.name: sha(p) for p in destination.glob('*.png')}))
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--scale', type=int, default=4)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.worker.resolve(), args.destination.resolve(), args.scale)
