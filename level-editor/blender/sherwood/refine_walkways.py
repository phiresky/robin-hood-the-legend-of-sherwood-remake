"""Separate timber deck boards, supporting beams and suspension rails."""
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import DATA, COS, collection, game_point, mesh, retire, source, tube

NAME='12 Walkways - remaining bridges and landings'
c=collection(NAME)
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())

def clip(poly,axis,limit,sign):
    out=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        da=(a.dot(axis)-limit)*sign;db=(b.dot(axis)-limit)*sign
        if da>=-1e-7:out.append(a)
        if da*db<0:out.append(a.lerp(b,da/(da-db)))
    return out

counts={}
for index,count,axis in [(93,28,Vector((.75,-.66,0))),(95,16,Vector((.72,.69,0))),
                         (96,9,Vector((.64,.77,0))),(99,8,Vector((.68,.73,0)))]:
    original=source(index)
    triangles=[[original.matrix_world@original.data.vertices[i].co for i in p.vertices]
               for p in original.data.polygons if (original.matrix_world.to_3x3()@p.normal).z>.8]
    if not triangles:raise RuntimeError(f'No top faces for {index}')
    lo=min(v.dot(axis) for tri in triangles for v in tri)
    hi=max(v.dot(axis) for tri in triangles for v in tri)
    for j in range(count):
        a=lo+(hi-lo)*(j+.02)/count;b=lo+(hi-lo)*(j+.98)/count
        bm=bmesh.new()
        for tri in triangles:
            poly=clip(clip(tri,axis,a,1),axis,b,-1)
            if len(poly)>=3:bm.faces.new([bm.verts.new(p) for p in poly])
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.001)
        bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=.0001)
        topfaces=list(bm.faces);boundary=[e for e in bm.edges if e.is_boundary]
        bottom={v:bm.verts.new(v.co-Vector((0,0,3.8))) for v in list(bm.verts)}
        for f in topfaces:bm.faces.new([bottom[v] for v in reversed(list(f.verts))])
        for e in boundary:
            u,v=e.verts;bm.faces.new((u,v,bottom[v],bottom[u]))
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bm.verts.index_update()
        obj=mesh(c,f'Deck {index:03} - board {j+1:02}',[v.co.copy() for v in bm.verts],[[v.index for v in f.verts] for f in bm.faces])
        bm.free()
        obj['source_obstacle']=f'building-{index:03}'
    retire(index,NAME);counts[index]=count

for index,chains,post_count in [(93,[(2,1),(3,0)],4),(95,[(0,1),(3,2)],2)]:
    corners=[game_point(p['x'],p['y'],p['z_top']) for p in level['sight_obstacles'][index]['points']]
    for side,(a,b) in enumerate(chains):
        p,q=corners[a],corners[b]
        tube(c,f'Bridge {index} underside beam {side}',[p-Vector((0,0,4)),q-Vector((0,0,4))],[2,2],8)
        # Bridge 95 is a short unrailed link in the image.
        if index==95:continue
        height=34/COS
        for j in range(post_count):
            bottom=p.lerp(q,j/(post_count-1))
            tube(c,f'Bridge {index} side {side} post {j}',[bottom-Vector((0,0,5)),bottom+Vector((0,0,height))],[1.65,1.4],8)
        for span in range(post_count-1):
            points=[]
            for j in range(13):
                t=j/12;u=(span+t)/(post_count-1)
                points.append(p.lerp(q,u)+Vector((0,0,height-7*math.sin(math.pi*t))))
            tube(c,f'Bridge {index} side {side} rope {span}',points,[.8]*len(points),8)
result={'deck_boards':counts,'objects':len(c.objects)}
