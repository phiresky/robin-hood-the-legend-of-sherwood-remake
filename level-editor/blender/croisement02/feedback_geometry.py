"""Source-specific corrections from the first Croisement02 geometry review."""
import math
import numpy as np
from mathutils import Vector
from scenery_geometry import Mesh,footprint,bevel
from tree_geometry import SIN,COS


def wall(obj,record,mask):
    c,u,v,length,width,height=footprint(record);points=record['points']
    coordinates=np.asarray([((Vector((p['x'],-p['y']/SIN,0))-c).dot(u),p['z_top']/COS) for p in points])
    order=np.argsort(coordinates[:,0]);lo=coordinates[order[:2]].mean(axis=0);hi=coordinates[order[-2:]].mean(axis=0)
    def top(t):
        q=c+u*t;x=round(q.x);fallback=float(np.interp(t,[lo[0],hi[0]],[lo[1],hi[1]]))
        if not 0<=x<mask.shape[1]:return fallback
        ys=np.flatnonzero(mask[:,x]);ground=-q.y*SIN
        ys=ys[(ys>ground-height*COS-18)&(ys<ground)]
        return max(2,(ground-float(ys.min())-abs(v.y)*width*.5*SIN)/COS) if len(ys) else fallback
    m=Mesh();steps=max(2,math.ceil(length/7));ts=np.linspace(-length/2,length/2,steps+1)
    for a,b in zip(ts[:-1],ts[1:]):
        za,zb=top(a),top(b)
        for z in np.arange(0,max(za,zb),7):
            if min(za,zb)<=z:continue
            aa,bb=a+.06,b-.06;low=z+.04;ha=min(z+6.94,za);hb=min(z+6.94,zb)
            if min(ha,hb)<=low+.02:continue
            start=len(m.vertices)
            for t,h in [(aa,ha),(bb,hb)]:
                for side,hz in [(-1,low),(1,low),(1,h),(-1,h)]:
                    m.vertices.append(tuple(c+u*t+v*side*width*.5+Vector((0,0,hz))))
            m.faces.extend(tuple(start+i for i in f) for f in [(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
    result=m.apply(obj);result.update(bevel(obj,.18,1));return result


def gate(obj,record):
    c,u,v,length,width,height=footprint(record);m=Mesh();up=Vector((0,0,1))
    # Two square end posts, three rails and the rising diagonal visible in art.
    for t in [-length/2,length/2]:m.box(c+u*t+up*height*.52,u,v,4.2,4.2,height*1.04)
    for z in [.15,.49,.82]:m.box(c+up*height*z,u,v,length-2,3.1,3.2)
    # Fix orientation in map space so the brace rises toward the right post.
    left,right=sorted([c-u*(length/2-2),c+u*(length/2-2)],key=lambda p:p.x)
    a=left+up*height*.15;b=right+up*height*.82
    direction=(b-a).normalized();cross=v.normalized();vertical=direction.cross(cross).normalized();start=len(m.vertices)
    for end in [a,b]:
        for s,t in [(-1,-1),(1,-1),(1,1),(-1,1)]:m.vertices.append(tuple(end+cross*s*1.5+vertical*t*1.6))
    m.faces.extend(tuple(start+i for i in f) for f in [(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
    return m.apply(obj)


def firewood(objects,mask):
    meshes=[Mesh(),Mesh()];up=Vector((0,0,1));front_a=Vector((1480,-244/SIN,0));front_b=Vector((1530,-237/SIN,0));across=(front_b-front_a).normalized();axis=Vector((-8,18/SIN,0)).normalized()
    for row,count in enumerate([8,7,6]):
        for column in range(count):
            t=(column+.5+(8-count)/2)/8;front=front_a.lerp(front_b,t)+up*(3.35+row*5.65)
            m=meshes[0 if front.x<1506 else 1];length=27+1.2*math.sin(column*2+row)
            side=axis.cross(up).normalized();normal=axis.cross(side).normalized()
            for candidate in np.arange(length,7,-.5):
                fits=True
                for j in range(12):
                    end=front+axis*candidate+3.1*(side*math.cos(j*math.tau/12)+normal*math.sin(j*math.tau/12))
                    x=round(end.x);ys=np.flatnonzero(mask[:,x])
                    if len(ys) and -end.y*SIN-end.z*COS<float(ys.min())+.5:fits=False;break
                if fits:length=float(candidate);break
            m.tube(front,front+axis*length,3.5,3.1,12)
    return [m.apply(o) for m,o in zip(meshes,objects)]


def kindling(objects):
    meshes=[Mesh(),Mesh()];base=Vector((1610,-230/SIN,0));up=Vector((0,0,1))
    for j in range(20):
        a=math.tau*j/20;d=Vector((math.cos(a)*15,math.sin(a)*13,0))
        start=base+d;end=base+d*.32+up*(35.5+1.2*math.sin(j*2.7))
        meshes[0 if j<10 else 1].tube(start,end,1.65,1.1,8)
    return [m.apply(o) for m,o in zip(meshes,objects)]

WATTLE_PROFILE=[(880,797),(893,803),(915,809),(935,816),(955,821),(978,825),(1000,835),(1020,842),(1040,845),(1060,852),(1080,858),(1100,865),(1120,870),(1140,877),(1160,880),(1180,886),(1200,891)]
WATTLE_POSTS=[(893,791),(914,793),(933,796),(956,801),(978,807),(1047,822),(1058,828),(1076,840),(1092,839),(1114,851),(1129,855),(1155,856)]

def wattle_top(x):
    if x<880:return 797+(x-880)*.30
    if x>1200:return 891+(x-1200)*.30
    return float(np.interp(x,*np.array(WATTLE_PROFILE).T))


def wattle(obj,record):
    c,u,v,length,width,height=footprint(record);m=Mesh();xmin=min(p['x'] for p in record['points']);xmax=max(p['x'] for p in record['points'])
    def point(x,screen_y,offset=0):
        t=(x-c.x)/u.x;p=c+u*t+v*offset;p.z=(-p.y*SIN-screen_y)/COS;return p
    for row in range(11):
        samples=np.linspace(xmin,xmax,max(2,round((xmax-xmin)/4)))
        for a,b in zip(samples[:-1],samples[1:]):
            m.tube(point(a,wattle_top(a)+row*2.15,math.sin(a*.16)*1.1),point(b,wattle_top(b)+row*2.15,math.sin(b*.16)*1.1),.8,n=6)
    posts=WATTLE_POSTS+[(x,wattle_top(x)-10) for x in [780,808,836,864,1180,1206,1235,1265,1295,1325,1355]]
    for x,top in posts:
        if xmin<=x<=xmax:
            end=point(x,top);start=end.copy();start.z=0;m.tube(start,end,1.8,1.45,8)
    return m.apply(obj)
