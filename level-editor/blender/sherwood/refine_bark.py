"""Replace cropped rear bark with tiled synthesis and continuous projection blending.

Run in Blender after the refinement passes. Generate the donor texture first:
texture-synthesis --tiling --threads 8 --seed 58 --out-size 128x512 \
  --out <pass2>/oak-bark-seamless.png generate <pass2>/oak-bark-broad-donor.png
The broader donor is Day map crop (931, 499, 1006, 585), excluding platforms.
Hidden bark remains inferred from the original oak crop.
"""
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from modeling import EYE, OUT, projection

NAME = 'Bark - synthesized wrap with blended source projection'
path = OUT / 'oak-bark-seamless.png'
if not path.is_file():
    raise FileNotFoundError(path)
backup = Path(bpy.data.filepath).with_name('sherwood-before-synthesized-bark.blend')
if not backup.exists():
    bpy.ops.wm.save_as_mainfile(filepath=str(backup), copy=True)

objects = list(bpy.data.collections['08 Refined forest - trunks roots and branches'].objects)
objects += [bpy.data.objects['Ladder oak - tapered fluted trunk']]
objects += [o for o in bpy.data.collections['09 Central treehouse - fork walls and thatch'].objects
            if o.name.startswith('Central oak -')]
baseline = bpy.data.collections['00 Baseline - original obstacle reconstruction']
baseline_meshes = {o.data for o in baseline.all_objects if o.type == 'MESH'}
if any(o.data in baseline_meshes for o in objects):
    raise RuntimeError('Refined bark unexpectedly shares a baseline mesh')

image = bpy.data.images.load(str(path), check_existing=False)
image.pack()
material = bpy.data.materials.get(NAME) or bpy.data.materials.new(NAME)
material.use_nodes = True
nodes = material.node_tree.nodes
links = material.node_tree.links
nodes.clear()
source = nodes.new('ShaderNodeTexImage')
source.image = next(n.image for n in bpy.data.materials['Sherwood measured Day projection'].node_tree.nodes
                    if n.type == 'TEX_IMAGE')
source.interpolation = 'Closest'
source.extension = 'EXTEND'
uv = nodes.new('ShaderNodeUVMap'); uv.uv_map = 'Bark source projection'
links.new(uv.outputs['UV'], source.inputs['Vector'])
bark = nodes.new('ShaderNodeTexImage'); bark.image = image
bark.extension = 'REPEAT'; bark.interpolation = 'Linear'
uv = nodes.new('ShaderNodeUVMap'); uv.uv_map = 'Bark continuous wrap'
links.new(uv.outputs['UV'], bark.inputs['Vector'])
weight = nodes.new('ShaderNodeVertexColor'); weight.layer_name = 'Bark source weight'
mix = nodes.new('ShaderNodeMixRGB'); mix.blend_type = 'MIX'
links.new(weight.outputs['Color'], mix.inputs[0])
links.new(bark.outputs['Color'], mix.inputs[1])
links.new(source.outputs['Color'], mix.inputs[2])
emission = nodes.new('ShaderNodeEmission')
links.new(mix.outputs[0], emission.inputs['Color'])
output = nodes.new('ShaderNodeOutputMaterial')
links.new(emission.outputs[0], output.inputs[0])

report = []
for obj in objects:
    mesh = obj.data
    # Crown refinement bisected several trunks, so their vertex indices no
    # longer describe ordered rings. Use geometric angles for every trunk.
    trunk = 'trunk' in obj.name
    sides = 48 if trunk else len(mesh.polygons[-1].vertices)
    if not trunk and (sides < 6 or len(mesh.vertices) % sides or len(mesh.polygons[-2].vertices) != sides):
        raise RuntimeError(f'Unexpected ring topology: {obj.name}')
    rings = len(mesh.vertices) // sides if not trunk else 1
    centers = [sum((mesh.vertices[r*sides+j].co for j in range(sides)), Vector()) / sides
               for r in range(rings)]
    lengths = [0.0]
    for a, b in zip(centers, centers[1:]):
        lengths.append(lengths[-1] + (b-a).length)
    center = sum((v.co for v in mesh.vertices), Vector()) / len(mesh.vertices)
    circumference = (math.tau * sum(math.hypot(v.co.x-center.x, v.co.y-center.y)
                                   for v in mesh.vertices)/len(mesh.vertices) if trunk else
                     sum((mesh.vertices[(rings//2)*sides+j].co -
                          mesh.vertices[(rings//2)*sides+(j+1)%sides].co).length for j in range(sides)))
    repeats = max(1, round(circumference / image.size[0]))
    project_uv = mesh.uv_layers.get('Bark source projection') or mesh.uv_layers.new(name='Bark source projection')
    wrap_uv = mesh.uv_layers.get('Bark continuous wrap') or mesh.uv_layers.new(name='Bark continuous wrap')
    weights = mesh.color_attributes.get('Bark source weight') or mesh.color_attributes.new(name='Bark source weight', type='FLOAT_COLOR', domain='POINT')
    for v in mesh.vertices:
        # Vertex interpolation avoids material-index and face-normal steps.
        facing = v.normal.dot(EYE)
        t = min(1.0, max(0.0, (facing - .15) / .5))
        u, w = projection(obj.matrix_world @ v.co)
        edge = min(u*1920, (1-u)*1920, w*1088, (1-w)*1088)
        t = t*t*(3-2*t) * min(1.0, max(0.0, edge / 8))
        weights.data[v.index].color = (t, t, t, 1)
    for face in mesh.polygons:
        angles = [math.atan2(mesh.vertices[i].co.y-center.y, mesh.vertices[i].co.x-center.x)
                  for i in face.vertices]
        if max(angles)-min(angles) > math.pi:
            angles = [a+math.tau if a<0 else a for a in angles]
        seam = any(i % sides == 0 for i in face.vertices) and any(i % sides == sides-1 for i in face.vertices)
        cap = len(face.vertices) == sides and face.index >= len(mesh.polygons)-2
        for li, angle in zip(face.loop_indices, angles):
            vi = mesh.loops[li].vertex_index
            ring, j = divmod(vi, sides)
            p = mesh.vertices[vi].co
            project_uv.data[li].uv = projection(obj.matrix_world @ p)
            if trunk:
                wrap_uv.data[li].uv = ((p.x-center.x)/128, (p.y-center.y)/512) if abs(face.normal.z)>.95 else (angle/math.tau*repeats, p.z/512)
            elif cap:
                delta = p-centers[ring]
                tangent = (centers[-1]-centers[0]).normalized()
                axis = tangent.cross(Vector((0, 1, 0))).normalized()
                other = tangent.cross(axis)
                wrap_uv.data[li].uv = (delta.dot(axis)/128, delta.dot(other)/512)
            else:
                # Integer wraps meet exactly at the ring closure; height is
                # continuous along the branch, never reset at each polygon.
                wrap_uv.data[li].uv = ((sides if seam and j == 0 else j)/sides*repeats,
                                      lengths[ring]/512)
        face.material_index = 0
    mesh.materials.clear()
    mesh.materials.append(material)
    obj['concealed_bark'] = '128x512 tiling texture-synthesis, continuous tube UVs, smooth source-facing blend'
    report.append({'object':obj.name,'rings':rings,'sides':sides,'wraps':repeats})

# TODO: Unseen bark identity cannot be recovered from a single source view.
result = {'objects':report, 'texture_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
          'baseline_meshes_untouched':len(baseline_meshes), 'backup':str(backup)}
(OUT/'bark-validation.json').write_text(json.dumps(result, indent=2))
bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
