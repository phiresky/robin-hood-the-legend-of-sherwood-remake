"""Shared geometry tools for source-registered Sherwood refinement."""

import math
from pathlib import Path

import bpy
from mathutils import Vector

from paths import ROOT, DATA, OUT
SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))
EYE = Vector((0, -COS, SIN))


def collection(name):
    if name in bpy.data.collections:
        raise RuntimeError(f'{name} already exists; inspect before rerunning')
    c = bpy.data.collections.new(name)
    for scene_name in ['Sherwood Refinement', 'Sherwood Animation Reference']:
        bpy.data.scenes[scene_name].collection.children.link(c)
    return c


def source(index):
    name = f'building-{index:03}'
    return next(o for o in bpy.data.collections['01 Refinement - working copy'].objects
                if o.get('source_obstacle') == name)


def retire(index, replacement):
    o = source(index)
    o.hide_render = True
    for scene in bpy.data.scenes:
        for layer in scene.view_layers:
            if o.name in layer.objects:
                o.hide_set(True, view_layer=layer)
    o['replaced_by'] = replacement


def projection(p):
    return (p[0] / 1920, 1 - (-p[1] * SIN - p[2] * COS) / 1088)


def game_point(x, y, z):
    return Vector((x, -y / SIN, z / COS))


def pixel_point(x, py, ground_y):
    return game_point(x, ground_y, ground_y - py)


def mesh(c, name, vertices, faces, material=None):
    data = bpy.data.meshes.new(name)
    data.from_pydata(vertices, [], faces)
    data.update()
    obj = bpy.data.objects.new(name, data)
    c.objects.link(obj)
    data.materials.append(material or bpy.data.materials['Sherwood measured Day projection'])
    uv = data.uv_layers.new(name='Original map projection')
    for loop in data.loops:
        uv.data[loop.index].uv = projection(data.vertices[loop.vertex_index].co)
    obj['reference'] = 'Sherwood Day map, 1920x1088; original 35 degree projection'
    return obj


def tube(c, name, points, radii, sides=12):
    points = [Vector(p) for p in points]
    if len(points) < 2 or len(points) != len(radii):
        raise ValueError('A tube needs matching point and radius arrays')
    closed = len(points)>3 and (points[0]-points[-1]).length<.0001
    if closed:
        points=points[:-1]
        radii=radii[:-1]
    vertices = []
    for i, (point, radius) in enumerate(zip(points, radii)):
        tangent = (points[(i+1)%len(points)]-points[(i-1)%len(points)] if closed
                   else points[min(i + 1, len(points) - 1)] - points[max(i - 1, 0)])
        if tangent.length < 1e-5:
            raise ValueError(f'{name} has a repeated tube station')
        tangent.normalize()
        normal = tangent.cross(Vector((0, 0, 1)))
        if normal.length < .001:
            normal = tangent.cross(Vector((1, 0, 0)))
        normal.normalize()
        bitangent = tangent.cross(normal).normalized()
        for j in range(sides):
            angle = math.tau * j / sides
            vertices.append(point + radius * (normal * math.cos(angle) + bitangent * math.sin(angle)))
    faces = [(i*sides+j, i*sides+(j+1)%sides, ((i+1)%len(points))*sides+(j+1)%sides, ((i+1)%len(points))*sides+j)
             for i in range(len(points) if closed else len(points)-1) for j in range(sides)]
    if not closed:
        faces += [tuple(reversed(range(sides))), tuple((len(points)-1)*sides+j for j in range(sides))]
    o = mesh(c, name, vertices, faces)
    for p in o.data.polygons:
        p.use_smooth = len(p.vertices) == 4
    return o
