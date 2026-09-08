"""Replace coarse hut skins with physical wall boards and roof shingles.

Existing measured wall planes, ridge slopes and door recesses remain registered.
Board divisions and thickness are inferred; painted windows remain textured.
"""
import math
import sys
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import collection,mesh,source,retire

NAME='14 Huts - timber walls and roof shingles'
c=collection(NAME)

def clip(poly,axis,limit,sign):
    out=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        da=(a.dot(axis)-limit)*sign;db=(b.dot(axis)-limit)*sign
        if da>=-1e-7:out.append(a)
        if da*db<0:out.append(a.lerp(b,da/(da-db)))
    return out

counts={}
for index in [0,1,2,8,9,10,104,105,107,108,119]:
    original=source(index)
    groups={}
    for face in original.data.polygons:
        normal=(original.matrix_world.to_3x3()@face.normal).normalized()
        if normal.z<-.1:continue
        polygon=[original.matrix_world@original.data.vertices[i].co for i in face.vertices]
        key=tuple(round(v,3) for v in normal)+(round(normal.dot(polygon[0]),1),)
        groups.setdefault(key,{'normal':normal,'polygons':[]})['polygons'].append(polygon)
    start=len(c.objects)
    for gi,group in enumerate(groups.values()):
        normal=group['normal'];polygons=group['polygons'];roof=normal.z>.25
        axis=normal.cross(Vector((0,0,1)))
        if axis.length<.1:axis=Vector((1,0,0))
        axis.normalize();other=normal.cross(axis).normalized()
        coords=[v.dot(axis) for p in polygons for v in p]
        lo,hi=min(coords),max(coords)
        count=max(1,math.ceil((hi-lo)/(7 if roof else 6)))
        cross=[v.dot(other) for p in polygons for v in p]
        low,high=min(cross),max(cross)
        rows=max(1,math.ceil((high-low)/23)) if roof else 1
        for j in range(count):
            for row in range(rows):
                a=lo+(hi-lo)*(j+.01)/count;b=lo+(hi-lo)*(j+.99)/count
                d=low+(high-low)*row/rows;e=low+(high-low)*(row+1)/rows
                bm=bmesh.new()
                for polygon in polygons:
                    poly=clip(clip(polygon,axis,a,1),axis,b,-1)
                    poly=clip(clip(poly,other,d,1),other,e,-1)
                    if len(poly)>=3:
                        bm.faces.new([bm.verts.new(p) for p in poly])
                bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.001)
                bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=.0001)
                if not bm.faces:
                    bm.free();continue
                top=list(bm.faces);edges=[edge for edge in bm.edges if edge.is_boundary]
                bottom={v:bm.verts.new(v.co-normal*(1.9 if roof else 2.1)) for v in list(bm.verts)}
                for f in top:bm.faces.new([bottom[v] for v in reversed(list(f.verts))])
                for edge in edges:
                    u,v=edge.verts;bm.faces.new((u,v,bottom[v],bottom[u]))
                bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.verts.index_update()
                label='roof shingle' if roof else 'wall plank'
                obj=mesh(c,f'Hut {index:03} {label} {gi}-{j}-{row}',[v.co.copy() for v in bm.verts],[[v.index for v in f.verts] for f in bm.faces])
                obj['source_obstacle']=f'building-{index:03}'
                obj['inferred']='Board divisions and thickness; measured surface planes retained'
                bm.free()
    retire(index,NAME);counts[index]=len(c.objects)-start
result={'collection':NAME,'pieces':counts,'total':len(c.objects)}
