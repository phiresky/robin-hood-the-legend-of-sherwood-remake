"""Give the camp's floating furniture slabs timber supports.

The support heights and footprint come from obstacles 11, 12 and 14-20.
TODO: these inferred four-leg arrangements need individual photographic/map
tracing; utensils, baskets, the fabric canopy and trestle joinery remain coarse.
"""

import json
import math
from pathlib import Path

import bpy
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
SIN=math.sin(math.radians(35))
COS=math.cos(math.radians(35))
NAME='06 Detail pass - camp furniture supports'
if NAME in bpy.data.collections:
    raise RuntimeError('Camp pass already exists')
collection=bpy.data.collections.new(NAME)
bpy.context.scene.collection.children.link(collection)
material=bpy.data.materials['Sherwood measured Day projection']
level=json.loads((ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.json').read_text())


def beam(name,a,b,radius):
    direction=b-a
    if direction.length<0.01:
        raise ValueError(f'Zero length support {name}')
    bpy.ops.mesh.primitive_cylinder_add(vertices=8,radius=radius,depth=direction.length,location=(a+b)/2)
    obj=bpy.context.object
    obj.name=name
    obj.rotation_euler=direction.to_track_quat('Z','Y').to_euler()
    collection.objects.link(obj)
    for c in list(obj.users_collection):
        if c!=collection:c.objects.unlink(obj)
    obj.data.materials.append(material)
    bpy.context.view_layer.update()
    uv=obj.data.uv_layers.active
    for loop in obj.data.loops:
        x,y,z=obj.matrix_world@obj.data.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(x/1920,1-(-y*SIN-z*COS)/1088)
    obj['inferred']='Support placement inferred from obstacle slab footprint'
    return obj


for index in [11,12,14,15,16,17,18,19,20]:
    points=level['sight_obstacles'][index]['points']
    if len(points)!=4:
        raise RuntimeError(f'Expected rectangular furniture obstacle {index}')
    corners=[Vector((p['x'],-p['y']/SIN,p['z_bottom']/COS)) for p in points]
    center=sum(corners,Vector())/4
    feet=[]
    for i,corner in enumerate(corners):
        top=corner.lerp(center,0.06)
        foot=Vector((top.x,top.y,0.5))
        beam(f'Camp {index:03} - leg {i+1}',foot,top,1.6 if index in (11,12,14) else 1.1)
        feet.append(foot.lerp(top,0.3))
    if index in (12,14):
        lengths=[(corners[(i+1)%4]-corners[i]).length for i in range(4)]
        shortest=min(range(4),key=lambda i:lengths[i])
        for i in [shortest,(shortest+2)%4]:
            beam(f'Camp {index:03} - cross stretcher {i}',feet[i],feet[(i+1)%4],1.2)
result={'collection':NAME,'supports':len(collection.objects)}
