"""Extend a surveyed root-bank cliff to its source-traced ground contact."""
import json
import math
import numpy as np
from catalog import OUT
from tree_geometry import replace_mesh,SIN,COS
from prepare_root_bank_domain import TOE


def build(obj):
    if obj['source_node']!='building-025':raise ValueError('Wrong bank receiver')
    native=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['sight_obstacles'][25]['points']
    def between(a,b,x):
        t=(x-a['x'])/(b['x']-a['x'])
        return dict(x=x,y=a['y']*(1-t)+b['y']*t,z_top=a['z_top']*(1-t)+b['z_top']*t)
    anchors=[native[0],between(native[0],native[1],1445),native[1],between(native[1],native[2],1403),
             *native[2:],dict(x=1405,y=185,z_top=.001),dict(x=1422,y=200,z_top=.001),dict(x=1450,y=208,z_top=.001)]
    toes=[TOE[0],(1445,235),TOE[1],(1403,250),*TOE[2:],(1405,185),(1422,200),(1450,208)]
    crest=np.asarray([(p['x'],-p['y']/SIN,p['z_top']/COS) for p in anchors])
    foot=np.asarray([(x,-y/SIN,0) for x,y in toes])
    # Dense cross sections keep a rounded toe and crest while the interior
    # cliff face stays steep; the exact source high-crest anchors are retained.
    rings=17;n=len(anchors);vertices=[]
    for r in range(rings):
        t=r/(rings-1);ease=t*t*(3-2*t)
        ring=foot*(1-t)+crest*t
        ring[:,2]=crest[:,2]*ease
        ring[6,0]-=6*math.sin(math.pi*t)
        vertices.extend(ring.tolist())
    faces=[]
    for r in range(rings-1):
        for i in range(n):
            j=(i+1)%n;faces.append((r*n+i,r*n+j,(r+1)*n+j,(r+1)*n+i))
    faces.append(tuple(reversed(range(n))))
    center=crest.mean(axis=0);center_index=len(vertices);vertices.append(center.tolist())
    for i in range(n):faces.append(((rings-1)*n+i,(rings-1)*n+(i+1)%n,center_index))
    result=replace_mesh(obj,vertices,faces,materials=list(obj.data.materials))
    for polygon in obj.data.polygons:polygon.use_smooth=True
    if result['nonmanifold_edges'] or result['degenerate_faces']:raise ValueError('Bank must remain closed and nondegenerate')
    result.update(source_node=obj['source_node'],native_height=float(crest[:,2].max()),source_toe=toes,
        source_profile=[(p['x'],p['y']-p['z_top']) for p in native],
        method='Native high-crest anchors with independent source toe, closed smooth cliff transitions',
        inference='Rear/lateral bank allocation and hidden base are inferred terrain partition; tree contact requires joint review')
    return result
