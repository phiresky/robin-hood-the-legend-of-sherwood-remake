"""Editable scenery construction from surveyed native footprints and heights.

Hidden cross sections, board counts, weave and masonry divisions are inferred.
These candidates require source silhouettes and oblique views to be reviewed.
"""
import math
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from tree_geometry import SIN,COS,replace_mesh


class Mesh:
    def __init__(self):self.vertices=[];self.faces=[]
    def tube(self,a,b,r0,r1=None,n=10):
        a,b=Vector(a),Vector(b);axis=b-a
        if axis.length<1e-5:return
        axis.normalize();helper=Vector((0,0,1)) if abs(axis.z)<.9 else Vector((1,0,0))
        u=axis.cross(helper).normalized();v=axis.cross(u).normalized();start=len(self.vertices)
        for center,r in [(a,r0),(b,r0 if r1 is None else r1)]:
            self.vertices.extend(tuple(center+r*(u*math.cos(i*math.tau/n)+v*math.sin(i*math.tau/n))) for i in range(n))
        self.faces.append(tuple(start+i for i in reversed(range(n))))
        self.faces.extend((start+i,start+(i+1)%n,start+n+(i+1)%n,start+n+i) for i in range(n))
        self.faces.append(tuple(start+n+i for i in range(n)))
    def box(self,center,u,v,du,dv,dz):
        center,u,v=Vector(center),Vector(u),Vector(v);start=len(self.vertices)
        for z in [-dz/2,dz/2]:
            for a,b in [(-1,-1),(1,-1),(1,1),(-1,1)]:self.vertices.append(tuple(center+u*a*du/2+v*b*dv/2+Vector((0,0,z))))
        self.faces.extend(tuple(start+i for i in f) for f in [(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
    def apply(self,obj):return replace_mesh(obj,self.vertices,self.faces,materials=list(obj.data.materials))


def footprint(record):
    pts=record['points'];xy=np.asarray([(p['x'],-p['y']/SIN) for p in pts]);center=xy.mean(axis=0)
    values,axes=np.linalg.eigh(np.cov((xy-center).T));u=axes[:,np.argmax(values)];v=np.array([-u[1],u[0]])
    along=(xy-center)@u;across=(xy-center)@v
    center+=u*(along.min()+along.max())/2+v*(across.min()+across.max())/2
    return Vector((*center,0)),Vector((*u,0)),Vector((*v,0)),float(np.ptp(along)),float(np.ptp(across)),float(np.mean([p['z_top'] for p in pts])/COS)


def bevel(obj,width,segments=2):
    bpy.context.view_layer.objects.active=obj
    modifier=obj.modifiers.new('Rounded exposed edges','BEVEL');modifier.width=width;modifier.segments=segments
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data)
    report=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    return report


def refine(obj,record,index,slug):
    c,u,v,length,width,height=footprint(record);m=Mesh();up=Vector((0,0,1))
    if index in [9,12] or 'fence' in slug:
        count=max(2,round(length/23)+1);positions=np.linspace(-length/2,length/2,count)
        for t in positions:m.tube(c+u*t,c+u*t+up*(height+3),2.1,1.65,8)
        if 'wattle' in slug or index==145:
            for row in range(max(4,round(height/4))):
                z=3+row*4
                for j in range(count-1):
                    a=c+u*positions[j]+v*((-1)**j*1.25)+up*z
                    b=c+u*positions[j+1]+v*((-1)**(j+1)*1.25)+up*(z+.35*math.sin(j+row))
                    m.tube(a,b,1.35,n=6)
        else:
            for z in [height*.25,height*.78]:m.box(c+up*z,u,v,length,3,4)
            m.tube(c-u*length/2+up*5,c+u*length/2+up*(height-5),1.6,n=6)
            if index==9:
                for t in np.linspace(-length/2,length/2,max(3,round(length/8))):m.box(c+u*t+up*height/2,u,v,3,3,height)
        method='Separate posts, rails and weave; member spacing inferred from source crop'
    elif 'stone-wall' in slug:
        courses=max(2,round(height/8))
        for j in range(courses):
            count=max(1,round(length/(14+(j%2)*4)));step=length/count
            for k in range(count):
                t=-length/2+(k+.5)*step;z=(j+.5)*height/courses
                m.box(c+u*t+up*z,u,v,step-.4,max(2,width-.2),height/courses-.35)
        report=m.apply(obj);report.update(bevel(obj,.7,1));report['method']='Separate irregular-course masonry within surveyed footprint';return report
    elif 'kindling' in slug:
        radius=max(3,min(length,width)/2)
        for j in range(14):
            angle=j*math.tau/14;d=u*math.cos(angle)+v*math.sin(angle)
            m.tube(c+d*radius,c+d*radius*.38+up*height,1.6,1.1,7)
        method='Tapered bundle of separate leaning sticks'
    elif 'firewood' in slug:
        radius=max(2,min(height/6,width/6));axis=u
        for row in range(3):
            for col in range(4-row):
                center=c+v*(col-(3-row)/2)*radius*2+up*radius*(1+row*1.7)
                m.tube(center-axis*length/2,center+axis*length/2,radius*.95,n=10)
        method='Three tiers of separate round logs'
    elif 'log' in slug and 'stump' not in slug:
        radius=max(2,height/2);center=c+up*radius
        m.tube(center-u*length/2,center+u*length/2,radius,radius*.82,12)
        method='Round tapered fallen log'
    elif 'stump' in slug or index==139:
        radius=max(2,min(length,width)*.48)
        m.tube(c,c+up*height*.22,radius*1.12,radius*.86,14)
        m.tube(c+up*height*.22,c+up*height,radius*.86,radius*.7,14)
        method='Flared stump with separate cut cap'
    elif index==138:
        p=[Vector((t['x'],-t['y']/SIN,t['z_top']/COS)) for t in record['points']]
        # Native roof is a single sloping quadrilateral; retain its slope.
        boards=24
        for j in range(boards):
            a=j/boards;b=(j+1)/boards
            q=[p[0].lerp(p[3],a),p[1].lerp(p[2],a),p[1].lerp(p[2],b),p[0].lerp(p[3],b)]
            start=len(m.vertices);m.vertices.extend(tuple(t-up*2) for t in q);m.vertices.extend(tuple(t) for t in q)
            m.faces.extend(tuple(start+i for i in f) for f in [(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
        for t in p:m.tube(Vector((t.x,t.y,0)),t,2.2,n=8)
        for edge in [0,2,3]:
            a,b=p[edge],p[(edge+1)%4];count=max(2,round((a-b).length/5))
            for j in range(count):
                top=a.lerp(b,(j+.5)/count);m.tube(Vector((top.x,top.y,1)),top-up*2,2,n=4)
        method='Individual roof boards, corner posts and three boarded sides; unobserved interior inferred'
    else:
        report=bevel(obj,4 if 'haystack' in slug else 2,3)
        report['method']='Rounded native ridge/rock edges; footprint and relief retained; fine surface sculpt remains pending'
        return report
    report=m.apply(obj);report['method']=method;return report
