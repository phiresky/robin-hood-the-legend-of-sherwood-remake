"""Replace confirmed tree occluders with tapered, rooted trunks and branches.

Footprint position and basic size follow the original obstacles. Tree identity
was checked against source crops. Rear bark uses a clean source bark sample;
depth, fluting, concealed branches and root cross-sections are inferred.
"""

import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from modeling import COS, DATA, EYE, OUT, SIN, collection, game_point, mesh, pixel_point, retire, tube

NAME = '08 Refined forest - trunks roots and branches'
c = collection(NAME)
level = json.loads((DATA / 'Levels/Sherwood.rhp.json').read_text())
ids = [25, 26, 27, 28, 29, 30, 31, 32, 34, 35, 36, 37, 40, 42, 44, 45, 46, 50, 51, 53, 54, 55]
stumps = {34, 40, 54}

# A clean bark patch from the central oak, without ladders or platforms.
donor_image = bpy.data.images.load(str(OUT / 'oak-bark-donor.png'), check_existing=True)
donor_image.pack()
donor = bpy.data.materials.new('Forest - concealed bark from source oak')
donor.use_nodes = True
nodes = donor.node_tree.nodes
nodes.clear()
texture = nodes.new('ShaderNodeTexImage')
texture.image = donor_image
emission = nodes.new('ShaderNodeEmission')
output = nodes.new('ShaderNodeOutputMaterial')
donor.node_tree.links.new(texture.outputs['Color'], emission.inputs['Color'])
donor.node_tree.links.new(emission.outputs[0], output.inputs[0])

tree_specs = {}
for index in ids:
    points = level['sight_obstacles'][index]['points']
    xs = [p['x'] for p in points]
    ys = [p['y'] for p in points]
    cx, cy = (min(xs)+max(xs))/2, (min(ys)+max(ys))/2
    width, depth = max(xs)-min(xs), (max(ys)-min(ys))/SIN
    height = max(p['z_top'] for p in points)
    if index not in stumps:
        height = max(height, cy + 40)
    if index == 34:
        height = 92
    if index == 32:
        # The central oak forks below the hut; a vertical extension would cut
        # through its interior. Its limbs are traced in refine_treehouse.py.
        height = 248
    shaft = .42 if index not in {29, 32, 37} else .47
    sections = [(0, .72, .70), (.025, .60, .59), (.085, .46, .46),
                (.24, shaft, shaft), (.52, shaft*.95, shaft*.95),
                (.78, shaft*.87, shaft*.87), (1, shaft*.80, shaft*.82)]
    vertices = []
    sides, rings = 48, 48
    for ring in range(rings):
        t = ring/(rings-1)
        for a, b in zip(sections, sections[1:]):
            if t <= b[0]:
                u = (t-a[0])/(b[0]-a[0])
                rx = width*(a[1]+(b[1]-a[1])*u)
                ry = depth*(a[2]+(b[2]-a[2])*u)
                break
        lean = (2.5*math.sin(t*1.7+index)-2.5*math.sin(index))*min(1,width/30)
        for j in range(sides):
            angle = j*math.tau/sides
            flute = min(1.2,width*.025)*(math.cos(angle*9+t)+.35*math.sin(angle*17))
            # Valleys between individual buttress roots prevent a circular
            # skirt at the ground contact in solid/oblique inspections.
            root_valley = .29*(1-max(0,math.cos(angle*6-index*.27))**4)*max(0,1-t/.09)
            x = cx + lean + (rx-width*root_valley+flute)*math.cos(angle)
            y = -cy/SIN + (ry-depth*root_valley+flute)*math.sin(angle)
            vertices.append((x,y,height*t/COS))
    faces = [(i*sides+j,i*sides+(j+1)%sides,(i+1)*sides+(j+1)%sides,(i+1)*sides+j)
             for i in range(rings-1) for j in range(sides)]
    faces += [tuple(reversed(range(sides))),tuple((rings-1)*sides+j for j in range(sides))]
    obj = mesh(c, f'Tree {index:03} - tapered trunk',vertices,faces)
    obj['source_obstacle'] = f'building-{index:03}'
    obj['inferred'] = 'Taper, elliptical cross-sections, concealed bark and longitudinal fluting'
    obj.data.materials.append(donor)
    for p in obj.data.polygons:
        p.use_smooth = len(p.vertices) == 4
        if p.normal.dot(EYE) < -.1:
            p.material_index = 1
            for li in p.loop_indices:
                v = obj.data.vertices[obj.data.loops[li].vertex_index].co
                angle = math.atan2(v.y+cy/SIN, v.x-cx)
                obj.data.uv_layers.active.data[li].uv = (angle*3/math.tau, v.z/95)
    retire(index, NAME)
    if index == 32:
        retire(48, NAME)  # two overlapping occluders represented the same oak
    tree_specs[index] = {'cx':cx,'cy':cy,'width':width,'height':height}
    # Buttress roots follow the lower visible tree mass rather than a flat cap.
    if width > 30 and index not in {40, 54}:
        for j in range(6):
            angle = math.tau*j/6 + index*.27
            length = width*(.65+.2*math.sin(j*2.1))
            base = Vector((cx,-cy/SIN,0))
            d = Vector((math.cos(angle),math.sin(angle),0))
            points = [base+d*width*.3+Vector((0,0,width*.6)),
                      base+d*width*.5+Vector((0,0,width*.2)),
                      base+d*length+Vector((0,0,2))]
            root = tube(c,f'Tree {index:03} - buttress root {j+1}',points,[width*.10,width*.075,.7],10)
            root['inferred'] = 'Root depth and rear layout; visible bark projected from map'

# Visible dead-wood branches traced in source pixel coordinates. The common
# ground-y plane retains their screen registration; subsequent depth offsets
# can be added along the camera ray without changing the reference silhouette.
traces = [
    (34,[(803,932,8),(799,901,7),(778,889,4),(754,878,1)]),
    (34,[(799,908,6),(814,887,3),(838,880,.7)]),
    (40,[(1327,352,11),(1307,321,9),(1286,294,5),(1278,264,1)]),
    (40,[(1310,325,7),(1320,299,4),(1338,278,1)]),
    (45,[(1671,160,10),(1645,139,8),(1604,126,6),(1586,106,1)]),
    (45,[(1660,149,7),(1650,109,5),(1627,83,1)]),
    (36,[(1772,329,10),(1763,292,8),(1738,259,4),(1710,243,1)]),
    (37,[(1857,427,16),(1871,375,10),(1888,350,2)]),
]
for index, trace in traces:
    cy=tree_specs[index]['cy']
    ps=[pixel_point(x,y,cy) for x,y,r in trace]
    obj=tube(c,f'Tree {index:03} - traced branch {trace[0][1]}',ps,[r for x,y,r in trace],12)
    obj['inferred']='Branch depth; silhouette traced from source Day map'

result = {'trees':len(ids),'meshes':len(c.objects),'collection':NAME}
