"""Identify native-camera crosspiece receivers without changing approved models."""
import hashlib
import json
import math
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement/restart2'
OUT=BASE/'hall-crosspiece-study-v1'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
direction=Vector((0,-c,s))
points=[(2795,515),(2800,515),(2805,512),(2810,508),(2820,500),(2830,491),
        (2795,535),(2805,535),(2820,530),(2765,530)]
records=[]
for model in [BASE/'hall-four-state-review-v1/approval-batch-v3/applied-applied.blend',
              BASE/'hall-textures-v1/applied-applied/bake-v1/model.blend']:
    bpy.ops.wm.open_mainfile(filepath=str(model))
    bpy.context.window.scene=bpy.data.scenes['york Refinement'];bpy.context.view_layer.update()
    objects=[o for o in bpy.data.collections['york Working'].all_objects if o.type=='MESH' and not o.hide_render]
    vertices=[];triangles=[];owners=[]
    for obj in objects:
        offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        obj.data.calc_loop_triangles()
        for tri in obj.data.loop_triangles:
            triangles.append(tuple(offset+i for i in tri.vertices));owners.append((obj,tri.polygon_index))
    tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True)
    hits=[]
    for x,y in points:
        origin=Vector((x,-y/s,0))+direction*10000
        hit,normal,index,distance=tree.ray_cast(origin,-direction)
        if index is None:raise ValueError('Missing native ray hit')
        obj,face=owners[index]
        hits.append({'pixel':[x,y],'object':obj.name,'source_node':obj.get('source_node'),
                     'asset_group':obj.get('asset_group'),'face':face,'world':list(hit)})
    detail=[]
    for o in bpy.data.collections['york Working'].all_objects:
        if o.type!='MESH' or o.get('source_node') not in {'building-790','building-791','building-810','building-831'}:continue
        world=[o.matrix_world@v.co for v in o.data.vertices]
        detail.append({'name':o.name,'node':o.get('source_node'),'group':o.get('asset_group'),
                       'hide_render':o.hide_render,'matrix':[list(row) for row in o.matrix_world],
                       'game_vertices':[[v.x,-v.y*s,v.z*c] for v in world],
                       'native_vertices':[[v.x,-v.y*s-v.z*c] for v in world],
                       'world_geometry_sha256':hashlib.sha256(json.dumps([list(v) for v in world]).encode()).hexdigest()})
    records.append({'model':str(model),'sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'hits':hits,'objects':detail})
OUT.mkdir(exist_ok=True)
(OUT/'receiver-diagnostic.json').write_text(json.dumps({'scope':'Read-only geometry/visibility diagnostic; no new source assignment','models':records},indent=2)+'\n')
print(OUT/'receiver-diagnostic.json')
