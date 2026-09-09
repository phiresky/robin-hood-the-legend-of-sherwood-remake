"""Faceted rock volumes and separate timber river fence from measured obstacles.

Rock footprints and heights follow sight obstacles; rounded cross-sections and
fracture relief are inferred from the only available view.
"""
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import DATA,COS,SIN,collection,game_point,mesh,retire,tube

NAME='13 Riverbank - boulders and timber fence'
c=collection(NAME)
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())

def rock(name,points,seed):
    center=sum((game_point(p['x'],p['y'],0) for p in points),Vector())/len(points)
    # Retain irregular original perimeter, add intermediate edges for chipped
    # corners and a rounded shoulder instead of a flat extruded polygon.
    controls=[]
    for a,b in zip(points,points[1:]+points[:1]):
        for t in [0,.5]:
            controls.append({k:a[k]*(1-t)+b[k]*t for k in ['x','y','z_bottom','z_top']})
    n=len(controls);verts=[]
    for ring,(t,scale) in enumerate([(0,.91),(.20,1),(.62,.96),(.90,.76),(1,.42)]):
        for j,p in enumerate(controls):
            base=game_point(p['x'],p['y'],0)
            relief=1+.035*math.sin(seed*2.1+j*2.4+ring*1.9)
            v=center+(base-center)*scale*relief
            v.z=(p['z_bottom']+(p['z_top']-p['z_bottom'])*t)/COS
            verts.append(v)
    faces=[]
    for r in range(4):
        for j in range(n):
            a=r*n+j;b=r*n+(j+1)%n;d=(r+1)*n+(j+1)%n;e=(r+1)*n+j
            faces.extend([(a,b,d),(a,d,e)])
    faces += [tuple(reversed(range(n))),tuple(4*n+j for j in range(n))]
    obj=mesh(c,name,verts,faces)
    obj['inferred']='Rock shoulder and fracture cross-sections; original footprint and height retained'
    return obj

for index in range(57,83):
    rock(f'Rock {index:03} - rounded fractured volume',level['sight_obstacles'][index]['points'],index)
    retire(index,NAME)

points=level['sight_obstacles'][121]['points'][:6]
for span,(a,b) in enumerate(zip(points,points[1:])):
    p,q=game_point(a['x'],a['y'],0),game_point(b['x'],b['y'],0)
    count=max(1,round((q-p).length/24))
    for j in range(count+1):
        if span and j==0:continue
        t=j/count;bottom=p.lerp(q,t)
        height=(a['z_top']*(1-t)+b['z_top']*t)/COS
        tube(c,f'River fence span {span} post {j}',[bottom,bottom+Vector((0,0,height+2))],[1.8,1.45],7)
    for rail in [.37,.79]:
        u=p+Vector((0,0,a['z_top']/COS*rail));v=q+Vector((0,0,b['z_top']/COS*rail))
        tube(c,f'River fence span {span} rail {rail}',[u,u.lerp(v,.5)+Vector((0,0,-.5)),v],[1.2,1.05,1.2],7)
retire(121,NAME)

# Individual dead branches replace the filled flat outline of the log piles.
for index,traces in [
    (116,[[(526,859,2),(551,843,3),(578,827,2)],[(533,864,2),(557,849,2),(579,832,1)]]),
    (117,[[(1795,978,4),(1840,1002,4),(1900,1040,2)],[(1809,974,3),(1852,991,3),(1911,1008,2)]]),
    (118,[[(1809,954,4),(1850,969,4),(1918,995,2)],[(1828,945,3),(1861,966,3),(1925,981,2)]]),
]:
    for j,trace in enumerate(traces):
        ps=[game_point(x,y+4,4) for x,y,r in trace]
        tube(c,f'Log pile {index} branch {j}',ps,[r for x,y,r in trace],10)
    retire(index,NAME)
result={'collection':NAME,'objects':len(c.objects),'rock_volumes':26}
