"""Run through Blender MCP after importing the Sherwood baseline and working copy.

First geometry pass: obstacle 94, the long suspension bridge. Source image
coordinates are measured on the 1920 x 1088 Day map, not on a resized preview.
TODO: recover concealed surfaces from additional references; projected colour
does not establish the shape or appearance of the reverse side.
"""

import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
from paths import DATA
SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))
NAME = '03 Detail pass - suspension bridge'
if NAME in bpy.data.collections:
    raise RuntimeError('Bridge detail collection already exists; inspect before rerunning')
work = bpy.data.collections['01 Refinement - working copy']
source = next(o for o in work.objects if o.get('source_obstacle') == 'building-094')
collection = bpy.data.collections.new(NAME)
bpy.context.scene.collection.children.link(collection)
image = bpy.data.images.load(str(DATA / 'Levels/Day/sherwood.map.png'), check_existing=True)
image.pack()
material = bpy.data.materials.new('Sherwood measured Day projection')
material.use_nodes = True
nodes = material.node_tree.nodes
nodes.clear()
tex = nodes.new('ShaderNodeTexImage')
tex.image = image
tex.extension = 'EXTEND'
emission = nodes.new('ShaderNodeEmission')
output = nodes.new('ShaderNodeOutputMaterial')
material.node_tree.links.new(tex.outputs['Color'], emission.inputs['Color'])
material.node_tree.links.new(emission.outputs[0], output.inputs['Surface'])


def project(p):
    return (p[0] / 1920, 1 - (-p[1] * SIN - p[2] * COS) / 1088)


def mesh(name, vertices, faces, uv_positions=None):
    data = bpy.data.meshes.new(name)
    data.from_pydata(vertices, [], faces)
    data.update()
    obj = bpy.data.objects.new(name, data)
    collection.objects.link(obj)
    data.materials.append(material)
    uv = data.uv_layers.new(name='Original map projection')
    positions = uv_positions if uv_positions is not None else vertices
    for polygon in data.polygons:
        for loop_index in polygon.loop_indices:
            index = data.loops[loop_index].vertex_index
            point = positions[index] if polygon.normal.z < -0.8 else vertices[index]
            uv.data[loop_index].uv = project(point)
    obj['reference'] = 'Sherwood Day 1920x1088; obstacle 94'
    return obj


def tube(name, points, radius, sides=8):
    points = [Vector(p) for p in points]
    vertices = []
    for i, p in enumerate(points):
        direction = (points[min(i + 1, len(points) - 1)] - points[max(0, i - 1)]).normalized()
        normal = direction.cross(Vector((0, 0, 1)))
        if normal.length < 0.01:
            normal = direction.cross(Vector((1, 0, 0)))
        normal.normalize()
        binormal = direction.cross(normal).normalized()
        for j in range(sides):
            angle = j * math.tau / sides
            vertices.append(p + radius * (math.cos(angle) * normal + math.sin(angle) * binormal))
    faces = []
    for i in range(len(points) - 1):
        for j in range(sides):
            k = (j + 1) % sides
            faces.append((i*sides+j, i*sides+k, (i+1)*sides+k, (i+1)*sides+j))
    faces.extend([tuple(reversed(range(sides))), tuple((len(points)-1)*sides+j for j in range(sides))])
    return mesh(name, vertices, faces)


level = json.loads((DATA / 'Levels/Sherwood.rhp.json').read_text())
points = level['sight_obstacles'][94]['points']
corners = [Vector((p['x'], -p['y']/SIN, p['z_top']/COS)) for p in points]
front = [corners[i] for i in (0, 6, 5, 4)]
back = [corners[i] for i in (1, 2, 3)]


def along(chain, t):
    lengths = [(b-a).length for a, b in zip(chain, chain[1:])]
    remaining = min(max(t, 0), 1) * sum(lengths)
    for i, length in enumerate(lengths):
        if remaining <= length or i == len(lengths)-1:
            return chain[i].lerp(chain[i+1], remaining/length)
        remaining -= length
    raise RuntimeError('Empty chain')


# Separate closed planks, preserving the measured end levels and curved plan.
for i in range(39):
    start, end = (i+0.035)/39, (i+0.965)/39
    top = [along(front, start), along(front, end), along(back, end), along(back, start)]
    bottom = [p-Vector((0, 0, 3.2)) for p in top]
    mesh(f'Bridge 094 - plank {i+1:02}', top+bottom,
         [(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)],
         top+top)


def at_pixel(px, py, chain):
    # Solve depth from the measured rail pixel and the corresponding deck edge.
    # x is monotonic on both obstacle chains.
    for a, b in zip(chain, chain[1:]):
        if a.x <= px <= b.x:
            q = a.lerp(b, (px-a.x)/(b.x-a.x))
            return Vector((px, q.y, (-q.y*SIN-py)/COS))
    q = chain[0] if px < chain[0].x else chain[-1]
    return Vector((px, q.y, (-q.y*SIN-py)/COS))


def measured_rail(name, pixels, chain):
    controls = [at_pixel(x, y, chain) for x, y in pixels]
    # Catmull-Rom interpolation follows the traced sag instead of imposing a
    # physically ideal catenary on the deliberately irregular original bridge.
    smooth = []
    for i in range(len(controls)-1):
        p0, p1 = controls[max(0,i-1)], controls[i]
        p2, p3 = controls[i+1], controls[min(i+2,len(controls)-1)]
        for j in range(8):
            t = j/8
            smooth.append(0.5*((2*p1)+(-p0+p2)*t+(2*p0-5*p1+4*p2-p3)*t*t+(-p0+3*p1-3*p2+p3)*t*t*t))
    smooth.append(controls[-1])
    tube(name, smooth, 0.85)


measured_rail('Bridge 094 - front hand rope', [(480,144),(514,163),(549,175),(566,180),(598,192),(628,200),(656,202)], front)
measured_rail('Bridge 094 - rear hand rope', [(514,126),(546,145),(579,155),(620,174),(665,181),(700,183),(729,179)], back)
for label, chain, posts in [
    ('front', front, [(480,140,176),(564,166,207),(658,202,235)]),
    ('rear', back, [(514,111,151),(580,147,185),(729,176,210)]),
]:
    for i, (x, top_y, bottom_y) in enumerate(posts):
        tube(f'Bridge 094 - {label} post {i+1}', [at_pixel(x,bottom_y,chain),at_pixel(x,top_y,chain)], 1.55, 10)
    lower = [along(chain,i/48)-Vector((0,0,3.5)) for i in range(49)]
    tube(f'Bridge 094 - {label} supporting rope', lower, 1.3)

source.hide_render = True
source.hide_set(True)
source['replaced_by'] = NAME
bpy.context.view_layer.update()
result = {'collection': NAME, 'objects': len(collection.objects), 'replaced': source.name}
