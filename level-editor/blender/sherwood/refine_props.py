"""Round cooperage, cooking pot, stools and spit, plus visible oak roots."""
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import DATA,COS,collection,game_point,mesh,pixel_point,retire,tube

NAME='16 Camp props and traced roots'
c=collection(NAME)
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())

def lathe(name,origin,axis,profile,sides=32):
    axis=Vector(axis).normalized();u=axis.cross(Vector((0,0,1)))
    if u.length<.01:u=axis.cross(Vector((0,1,0)))
    u.normalize();v=axis.cross(u).normalized()
    vertices=[origin+axis*h+r*(u*math.cos(j*math.tau/sides)+v*math.sin(j*math.tau/sides)) for h,r in profile for j in range(sides)]
    faces=[(i*sides+j,i*sides+(j+1)%sides,(i+1)*sides+(j+1)%sides,(i+1)*sides+j) for i in range(len(profile)-1) for j in range(sides)]
    faces.extend([tuple(reversed(range(sides))),tuple((len(profile)-1)*sides+j for j in range(sides))])
    obj=mesh(c,name,vertices,faces)
    for p in obj.data.polygons:p.use_smooth=len(p.vertices)==4
    return obj,u,v

for index in [5,6,21,120]:
    points=level['sight_obstacles'][index]['points']
    cx=sum(p['x'] for p in points)/len(points);cy=sum(p['y'] for p in points)/len(points)
    bottom=min(p['z_bottom'] for p in points);height=(max(p['z_top'] for p in points)-bottom)/COS
    radius=(max(p['x'] for p in points)-min(p['x'] for p in points))*.43
    origin=game_point(cx,cy,bottom)
    obj,u,v=lathe(f'Cooperage {index:03} - bulging staves',origin,(0,0,1),[(0,radius*.83),(height*.2,radius*.96),(height*.5,radius),(height*.8,radius*.96),(height,radius*.83)],24)
    for j,t in enumerate([.12,.32,.75,.93]):
        r=radius*(.88+.12*math.sin(math.pi*t))+.15
        ps=[origin+Vector((0,0,height*t))+r*(u*math.cos(k*math.tau/32)+v*math.sin(k*math.tau/32)) for k in range(33)]
        tube(c,f'Cooperage {index:03} hoop {j}',ps,[.4]*33,6)
    retire(index,NAME)

# The large supply barrel lies on its side, not upright like its box obstacle.
p=level['sight_obstacles'][56]['points']
a=game_point((p[0]['x']+p[1]['x'])/2,(p[0]['y']+p[1]['y'])/2,26.5)
b=game_point((p[2]['x']+p[3]['x'])/2,(p[2]['y']+p[3]['y'])/2,26.5)
axis=(b-a).normalized();length=(b-a).length
obj,u,v=lathe('Supply barrel - horizontal bowed staves',a,axis,[(0,16),(length*.15,19),(length*.5,21),(length*.85,19),(length,16)],40)
for j,t in enumerate([.05,.22,.78,.95]):
    r=16+5*math.sin(math.pi*t)
    ps=[a+axis*length*t+r*(u*math.cos(k*math.tau/48)+v*math.sin(k*math.tau/48)) for k in range(49)]
    tube(c,f'Supply barrel iron hoop {j}',ps,[.85]*49,8)
retire(56,NAME)

points=level['sight_obstacles'][112]['points']
cx=sum(p['x'] for p in points)/4;cy=sum(p['y'] for p in points)/4
origin=game_point(cx,cy,0)
lathe('Cooking cauldron - hollow bowl',origin,(0,0,1),[(0,4),(2,9),(8,12),(15,10),(17,9),(17,7.8),(14,8.8),(8,10.7),(3,7),(2.2,3)],40)
ps=[origin+Vector((10*math.cos(k*math.pi/24),0,17+13*math.sin(k*math.pi/24))) for k in range(25)]
tube(c,'Cooking cauldron - arched handle',ps,[.7]*25,8)
retire(112,NAME)

for index in [23,113]:
    ps=level['sight_obstacles'][index]['points']
    cx=sum(p['x'] for p in ps)/4;cy=sum(p['y'] for p in ps)/4
    height=max(p['z_top'] for p in ps)/COS;radius=(max(p['x'] for p in ps)-min(p['x'] for p in ps))*.49
    center=game_point(cx,cy,0)
    lathe(f'Stool {index} - round seat',center,(0,0,1),[(height-2,radius),(height,radius)],32)
    for j in range(3):
        direction=Vector((math.cos(j*math.tau/3),math.sin(j*math.tau/3),0))
        tube(c,f'Stool {index} leg {j}',[center+direction*radius*.83,center+direction*radius*.52+Vector((0,0,height-1))],[.8,1],8)
    retire(index,NAME)

a=game_point(458,803,30);b=game_point(493,796,30)
axis=(b-a).normalized();length=(b-a).length
lathe('Roast - rounded body',a,axis,[(0,1.3),(length*.17,4),(length*.38,7.8),(length*.73,8),(length*.94,5),(length,2)],24)
tube(c,'Cooking spit - iron shaft',[a-axis*7,b+axis*8],[.6,.6],8)
for j,p in enumerate([a-axis*6,b+axis*7]):
    tube(c,f'Cooking spit upright {j}',[Vector((p.x,p.y,0)),p+Vector((0,0,7))],[.7,.7],8)
for j,t in enumerate([.37,.77]):
    p=a.lerp(b,t)
    tube(c,f'Roast hanging leg {j}',[p+Vector((0,-5,-2)),p+Vector((3,-7,-10)),p+Vector((6,-7,-9))],[2,1.4,.7],10)
retire(114,NAME)

for name,trace in [
    ('Central oak left buttress',[(955,568,65,13),(933,604,32,10),(909,631,10,6),(880,655,1,1)]),
    ('Central oak left root',[(956,596,35,10),(923,617,14,7),(886,620,1,1)]),
]:
    tube(c,name,[game_point(x,y+h,h) for x,y,h,r in trace],[r for x,y,h,r in trace],14)
retire(33,NAME);retire(52,NAME)
tube(c,'Tree 036 upper traced limb',[pixel_point(x,y,510) for x,y in [(1772,329),(1761,285),(1754,232)]],[10,8,5],16)
retire(38,NAME)
result={'collection':NAME,'objects':len(c.objects)}
