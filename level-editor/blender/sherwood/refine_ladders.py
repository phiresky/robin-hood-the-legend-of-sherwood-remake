"""Remove overlapping ladder panels and finish concealed bark UVs."""
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import DATA,EYE,COS,SIN,collection,game_point,pixel_point,projection,retire,tube

NAME='17 Ladder cleanup and concealed bark'
c=collection(NAME)
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())
# Zero-thickness ladder obstacles 97, 98 and 101 were omitted by the exporter.
# Their visible rungs are reconstructed in this pass and refine_treehouse.py.
ps=[game_point(p['x'],p['y'],p['z_top']) for p in level['sight_obstacles'][100]['points']]
for label,a,b in [('right',ps[0],ps[1]),('left',ps[3],ps[2])]:
    tube(c,'Left treehouse ladder '+label,[a,b],[1.2,1.2],8)
for j in range(13):
    t=(j+.5)/13
    tube(c,f'Left treehouse ladder rung {j+1}',[ps[0].lerp(ps[1],t),ps[3].lerp(ps[2],t)],[1.1,1.1],8)
retire(100,NAME)
for label,left,right,count in [
    ('above platform',[(423,124),(418,153)],[(443,127),(438,156)],4),
    ('below platform',[(419,184),(417,209)],[(435,186),(433,211)],3),
]:
    # Place the ladder just outside the front trunk, using its measured depth.
    a,b=[pixel_point(x,y,426) for x,y in left]
    d,e=[pixel_point(x,y,426) for x,y in right]
    for side,u,v in [('left',a,b),('right',d,e)]:
        tube(c,f'Ladder oak {label} {side} stile',[u,v],[.85,.85],8)
    for j in range(count):
        t=(j+.5)/count
        tube(c,f'Ladder oak {label} rung {j+1}',[a.lerp(b,t),d.lerp(e,t)],[1.1,1.1],8)

donor=next(m for m in bpy.data.materials if m.name.startswith('Forest - concealed bark from source oak'))
objects=list(bpy.data.collections['08 Refined forest - trunks roots and branches'].objects)
objects += [bpy.data.objects['Ladder oak - tapered fluted trunk']]
objects += [o for o in bpy.data.collections['09 Central treehouse - fork walls and thatch'].objects if o.name.startswith('Central oak -')]
changed=0
for obj in objects:
    if donor.name not in obj.data.materials:obj.data.materials.append(donor)
    slot=obj.data.materials.find(donor.name)
    cx=sum(v.co.x for v in obj.data.vertices)/len(obj.data.vertices)
    cy=sum(v.co.y for v in obj.data.vertices)/len(obj.data.vertices)
    for face in obj.data.polygons:
        uv=[projection(obj.data.vertices[i].co) for i in face.vertices]
        outside=all(v<0 or v>1 or u<0 or u>1 for u,v in uv)
        if face.normal.dot(EYE)<=0 or outside:
            face.material_index=slot;changed+=1
            angles=[math.atan2(obj.data.vertices[obj.data.loops[li].vertex_index].co.y-cy,
                               obj.data.vertices[obj.data.loops[li].vertex_index].co.x-cx) for li in face.loop_indices]
            if max(angles)-min(angles)>math.pi:angles=[a+math.tau if a<0 else a for a in angles]
            for li,angle in zip(face.loop_indices,angles):
                v=obj.data.vertices[obj.data.loops[li].vertex_index].co
                if len(face.vertices)>20:
                    # Planar caps must not collapse to a single texture row.
                    obj.data.uv_layers.active.data[li].uv=((v.x-cx)/60+.5,(v.y-cy)/60+.5)
                else:
                    obj.data.uv_layers.active.data[li].uv=(angle*3/math.tau,v.z/95)
    obj['concealed_bark']='Clean source oak crop on hidden/out-of-map faces; depth remains inferred'
# Huts cut by the map border need valid wood UVs rather than an infinitely
# stretched edge pixel. Reuse a clean vertical timber crop from the Day map.
wood=bpy.data.materials['Sherwood measured Day projection'].copy()
wood.name='Huts - concealed source timber'
nodes=wood.node_tree.nodes;links=wood.node_tree.links
tex=next(n for n in nodes if n.type=='TEX_IMAGE')
uv=nodes.new('ShaderNodeTexCoord')
wrap=nodes.new('ShaderNodeVectorMath');wrap.operation='FRACTION'
scale=nodes.new('ShaderNodeVectorMath');scale.operation='MULTIPLY'
scale.inputs[1].default_value=(15/1920,32/1088,1)
offset=nodes.new('ShaderNodeVectorMath');offset.operation='ADD'
offset.inputs[1].default_value=(553/1920,1-393/1088,0)
links.new(uv.outputs['UV'],wrap.inputs[0]);links.new(wrap.outputs[0],scale.inputs[0])
links.new(scale.outputs[0],offset.inputs[0]);links.new(offset.outputs[0],tex.inputs['Vector'])
wood_faces=0
for obj in bpy.data.collections['14 Huts - timber walls and roof shingles'].objects:
    obj.data.materials.append(wood);slot=len(obj.data.materials)-1
    for face in obj.data.polygons:
        projected=[projection(obj.data.vertices[i].co) for i in face.vertices]
        outside=any(u<0 or u>1 or v<0 or v>1 for u,v in projected)
        if outside or face.normal.dot(EYE)<-.05:
            face.material_index=slot;wood_faces+=1
            for li in face.loop_indices:
                v=obj.data.vertices[obj.data.loops[li].vertex_index].co
                obj.data.uv_layers.active.data[li].uv=((v.x+v.y)/35,v.z/55 if abs(face.normal.z)<.5 else (v.x-v.y)/55)
result={'collection':NAME,'ladder_parts':len(c.objects),'bark_faces':changed,'concealed_timber_faces':wood_faces}
