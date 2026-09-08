"""Second Sherwood pass: round the ladder oak and give its ladder real depth.

The camera-facing outline is traced from the Day map and registered against
obstacle 24. Cross-section depth and concealed bark are inferred.
TODO: refine roots, upper branches, rear bark, and platform joinery from more
references. This is a reviewable first pass, not a fidelity-complete asset.
"""

import math

import bpy
from mathutils import Vector

SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))
NAME = '04 Detail pass - ladder oak'
if NAME in bpy.data.collections:
    raise RuntimeError('Oak pass already exists; inspect before rerunning')
collection = bpy.data.collections.new(NAME)
bpy.context.scene.collection.children.link(collection)
material = bpy.data.materials['Sherwood measured Day projection']
source = next(o for o in bpy.data.collections['01 Refinement - working copy'].objects
              if o.get('source_obstacle') == 'building-024')


def mesh(name, vertices, faces, uv_positions=None):
    data = bpy.data.meshes.new(name)
    data.from_pydata(vertices, [], faces)
    data.update()
    obj = bpy.data.objects.new(name, data)
    collection.objects.link(obj)
    data.materials.append(material)
    uv = data.uv_layers.new(name='Original map projection')
    positions = vertices if uv_positions is None else uv_positions
    for loop in data.loops:
        x, y, z = positions[loop.vertex_index]
        uv.data[loop.index].uv = (x/1920, 1-(-y*SIN-z*COS)/1088)
    obj['reference'] = 'Sherwood Day 1920x1088; obstacle 24'
    return obj


def tube(name, points, radius):
    vertices = []
    sides = 8
    for i, p in enumerate(points):
        direction = (points[min(i+1,len(points)-1)]-points[max(i-1,0)]).normalized()
        normal = direction.cross(Vector((0,0,1)))
        if normal.length < 0.01:
            normal = direction.cross(Vector((1,0,0)))
        normal.normalize()
        other = direction.cross(normal).normalized()
        for j in range(sides):
            a = j*math.tau/sides
            vertices.append(p+radius*(math.cos(a)*normal+math.sin(a)*other))
    faces = [(i*sides+j,i*sides+(j+1)%sides,(i+1)*sides+(j+1)%sides,(i+1)*sides+j)
             for i in range(len(points)-1) for j in range(sides)]
    faces += [tuple(reversed(range(sides))),tuple((len(points)-1)*sides+j for j in range(sides))]
    return mesh(name,vertices,faces)


# game height, centre x, centre ground y, radius x, radius in true ground y
profile = [(0,428,409,40,37),(12,427,409,34,31),(45,425,409,27,26),
           (95,424,409,25,24),(160,424,409,26,25),(225,425,409,28,26),
           (275,423,409,24,24),(315,420,409,25,24),(355,417,409,27,25),
           (400,417,409,29,27),(470,420,409,30,27),(530,422,409,31,28)]


def section(height):
    for a,b in zip(profile,profile[1:]):
        if height <= b[0]:
            t = max(0,(height-a[0])/(b[0]-a[0]))
            return [x+(y-x)*t for x,y in zip(a,b)]
    return profile[-1]


vertices, uv_positions, faces = [], [], []
sides, rings = 64, 72
for i in range(rings):
    height = profile[-1][0]*i/(rings-1)
    h,cx,cy,rx,ry = section(height)
    for j in range(sides):
        a = j*math.tau/sides
        # Shallow longitudinal bark flutes and a flared, irregular root collar.
        flute = (0.7+1.1*math.exp(-h/30))*math.cos(9*a+0.003*h)+0.35*math.cos(17*a)
        x = cx+(rx+flute)*math.cos(a)
        y = -cy/SIN+(ry+flute)*math.sin(a)
        z = h/COS
        vertices.append((x,y,z))
        # Concealed hemisphere borrows the matching front height; it cannot be
        # recovered from this single image. Keep that limitation explicit.
        uv_positions.append((x,-cy/SIN-abs((ry+flute)*math.sin(a)),z))
for i in range(rings-1):
    for j in range(sides):
        faces.append((i*sides+j,i*sides+(j+1)%sides,(i+1)*sides+(j+1)%sides,(i+1)*sides+j))
faces += [tuple(reversed(range(sides))),tuple((rings-1)*sides+j for j in range(sides))]
trunk = mesh('Ladder oak - tapered fluted trunk',vertices,faces,uv_positions)
trunk['inferred'] = 'Elliptical depth, bark fluting, concealed hemisphere'
for polygon in trunk.data.polygons:
    polygon.use_smooth = len(polygon.vertices)==4
    # Sloping rear surfaces can still face the elevated original camera. Keep
    # their true projection instead of borrowing a front-hemisphere UV.
    if polygon.normal.dot(Vector((0,-COS,SIN)))>0 or len(polygon.vertices)>4:
        for loop_index in polygon.loop_indices:
            x,y,z=vertices[trunk.data.loops[loop_index].vertex_index]
            trunk.data.uv_layers.active.data[loop_index].uv=(x/1920,1-(-y*SIN-z*COS)/1088)


def surface_point(x,py):
    def point(h):
        _,cx,cy,rx,ry = section(h)
        fraction=(x-cx)/rx
        if abs(fraction)>1:
            raise ValueError(f'Ladder pixel x={x} lies outside trunk at height {h}')
        return Vector((x,-cy/SIN-ry*math.sqrt(1-fraction*fraction)-1.4,h/COS))
    lo,hi=0.0,355.0
    for _ in range(36):
        h=(lo+hi)/2;p=point(h)
        projected=-p.y*SIN-p.z*COS
        if projected>py:lo=h
        else:hi=h
    return point((lo+hi)/2)


left=[(417,208),(414,254),(410,308),(408,360),(404,420)]
right=[(433,208),(430,254),(425,308),(422,360),(420,427)]


def ladder_x(chain,y):
    for a,b in zip(chain,chain[1:]):
        if y<=b[1]:
            t=max(0,(y-a[1])/(b[1]-a[1]))
            return a[0]+(b[0]-a[0])*t
    return chain[-1][0]


for label,chain in [('left',left),('right',right)]:
    ps=[surface_point(ladder_x(chain,y),y) for y in range(208,421,3)]
    tube('Ladder oak - '+label+' rope stile',ps,0.85)
for i,y in enumerate(range(215,421,10)):
    a=surface_point(ladder_x(left,y)-1,y)
    b=surface_point(ladder_x(right,y)+1,y+1)
    tube(f'Ladder oak - rung {i+1:02}',[a,b],1.1)

source.hide_render=True
source.hide_set(True)
source['replaced_by']=NAME
bpy.context.view_layer.update()
result={'collection':NAME,'objects':len(collection.objects),'trunk_vertices':len(vertices)}
