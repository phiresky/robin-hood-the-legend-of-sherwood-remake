"""CPU-only connected wood shell and matched-boundary collar primitives.

Projected source coordinates determine the front outline. A two-dimensional
Poisson field supplies inferred thickness, independently of row extrema.
No existing Blender mesh is loaded or modified by this module.
"""
import math
from collections import Counter,defaultdict
import numpy as np
from scipy.ndimage import binary_fill_holes,label
from scipy.sparse import coo_matrix
from scipy.sparse.linalg import spsolve
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
RAY=np.array([0.,-COS,SIN])

def smooth_positive(value,band=2.):
    value=np.asarray(value,float);q=np.clip((value+band)/(2*band),0,1)
    return np.where(value<=-band,0,np.where(value>=band,value,2*band*(q**3-.5*q**4)))

def center_depth(y,ground):
    """Ground the source-facing boundary smoothly; inferred rear may be buried."""
    height=.25+smooth_positive((ground-np.asarray(y))/COS-.25)
    return (height+np.asarray(y)*COS)/SIN

def thickness_field(observed):
    labels,count=label(observed)
    if not count:raise ValueError('Empty source domain')
    sizes=np.bincount(labels.ravel());sizes[0]=0
    body=binary_fill_holes(labels==int(np.argmax(sizes)))
    indices=np.full(body.shape,-1,int);indices[body]=np.arange(body.sum())
    rr=[];cc=[];vv=[]
    for y,x in zip(*np.where(body)):
        i=int(indices[y,x]);rr.append(i);cc.append(i);vv.append(4.)
        for dy,dx in [(0,1),(0,-1),(1,0),(-1,0)]:
            yy,xx=y+dy,x+dx
            if 0<=yy<body.shape[0] and 0<=xx<body.shape[1] and body[yy,xx]:
                rr.append(i);cc.append(int(indices[yy,xx]));vv.append(-1.)
    matrix=coo_matrix((vv,(rr,cc)),shape=(int(body.sum()),)*2).tocsr()
    potential=np.zeros(body.shape);solution=spsolve(matrix,np.ones(int(body.sum())));potential[body]=solution
    if not np.all(np.isfinite(solution)) or solution.min()<=0:raise ValueError('Invalid positive thickness field')
    return body,np.sqrt(2*potential),dict(observed_pixels=int(observed.sum()),main_body_observed_pixels=int((body&observed).sum()),unresolved_disconnected_pixels=int((observed&~body).sum()),filled_interior_pixels=int((body&~observed).sum()),poisson_residual_max=float(np.abs(matrix@solution-1).max()))

def shell(body,thickness,origin,ground):
    """One closed branching shell; no separate parent/root caps or unions.

The temporary outer rim includes the top of the CPU study crop. Integration
must remove that top closure and attach a matched retained boundary collar.
Diagonal pixel contacts get separate vertex fans, avoiding nonmanifold nodes.
"""
    vertices=[];faces=[];corner_ids={};edge_ids={};cells=set(zip(*np.where(body)))
    def point(x,y,offset):
        sx=x+origin[0];sy=y+origin[1]
        return np.array([sx,-sy*SIN,-sy*COS])+RAY*(float(center_depth(sy,ground))+offset)
    for cy,cx in sorted({(y+dy,x+dx) for y,x in cells for dy,dx in [(0,0),(0,1),(1,1),(1,0)]}):
        adjacent={(y,x) for y,x in [(cy-1,cx-1),(cy-1,cx),(cy,cx-1),(cy,cx)] if (y,x) in cells}
        remaining=set(adjacent)
        while remaining:
            group={remaining.pop()};stack=list(group)
            while stack:
                y,x=stack.pop()
                for neighbour in [(y-1,x),(y+1,x),(y,x-1),(y,x+1)]:
                    if neighbour in remaining:remaining.remove(neighbour);group.add(neighbour);stack.append(neighbour)
            boundary=len(group)<4;radius=0. if boundary else float(np.mean([thickness[y,x] for y,x in group]))
            front=len(vertices);vertices.append(point(cx,cy,radius))
            back=front if boundary else len(vertices)
            if not boundary:vertices.append(point(cx,cy,-radius))
            for cell in group:corner_ids[(cell,cy,cx)]=(front,back)
    for y,x in sorted(cells):
        for endpoints,other in [(((y,x),(y,x+1)),(y-1,x)),(((y,x+1),(y+1,x+1)),(y,x+1)),(((y+1,x),(y+1,x+1)),(y+1,x)),(((y,x),(y+1,x)),(y,x-1))]:
            key=tuple(sorted(endpoints))
            if key in edge_ids:continue
            boundary=other not in cells;radius=0. if boundary else float((thickness[y,x]+thickness[other])/2)
            (ay,ax),(by,bx)=key;front=len(vertices);vertices.append(point((ax+bx)/2,(ay+by)/2,radius));back=front if boundary else len(vertices)
            if not boundary:vertices.append(point((ax+bx)/2,(ay+by)/2,-radius))
            edge_ids[key]=(front,back)
    for y,x in sorted(cells):
        front=len(vertices);vertices.append(point(x+.5,y+.5,float(thickness[y,x])))
        back=len(vertices);vertices.append(point(x+.5,y+.5,-float(thickness[y,x])))
        perimeter=[];coordinates=[(y,x),(y,x+1),(y+1,x+1),(y+1,x)]
        for i,corner in enumerate(coordinates):
            following=coordinates[(i+1)%4];perimeter.extend([corner_ids[((y,x),*corner)],edge_ids[tuple(sorted((corner,following)))]])
        for i in range(8):
            j=(i+1)%8;faces.append((front,perimeter[j][0],perimeter[i][0]));faces.append((back,perimeter[i][1],perimeter[j][1]))
    return np.asarray(vertices),np.asarray(faces,int)

def topology(vertices,faces):
    edges=Counter();directions=defaultdict(int)
    for tri in faces:
        for a,b in zip(tri,np.roll(tri,-1)):
            key=tuple(sorted((int(a),int(b))));edges[key]+=1;directions[key]+=1 if a<b else -1
    area=np.linalg.norm(np.cross(vertices[faces[:,1]]-vertices[faces[:,0]],vertices[faces[:,2]]-vertices[faces[:,0]]),axis=1)/2
    return dict(vertices=len(vertices),triangles=len(faces),nonmanifold_edges=sum(v!=2 for v in edges.values()),inconsistent_oriented_edges=sum(v!=0 for v in directions.values()),degenerate_triangles=int((area<1e-9).sum()),minimum_triangle_area=float(area.min()))

def collar(lower,upper,lower_tangent,upper_tangent,steps=16):
    """Cubic Hermite rows with exact endpoints and explicitly matched tangents.

Inputs must have matching ordered loop correspondence established from real
retained boundary topology. This function never sorts vertices by angle or
assumes a single star-shaped loop.
"""
    arrays=[np.asarray(a,float) for a in [lower,upper,lower_tangent,upper_tangent]]
    if any(a.shape!=arrays[0].shape for a in arrays) or arrays[0].ndim!=2 or arrays[0].shape[1]!=3:raise ValueError('Mismatched boundary loops/tangents')
    lo,hi,m0,m1=arrays;t=np.linspace(0,1,steps+1)[:,None,None]
    return (2*t**3-3*t**2+1)*lo+(t**3-2*t**2+t)*m0+(-2*t**3+3*t**2)*hi+(t**3-t**2)*m1

def ordered_boundary_loops(edges):
    """Walk actual mesh edges; reject branched/open cuts instead of angle-sorting."""
    neighbours=defaultdict(set)
    for a,b in edges:
        a,b=int(a),int(b)
        if a==b:raise ValueError('Degenerate boundary edge')
        neighbours[a].add(b);neighbours[b].add(a)
    if not neighbours or any(len(v)!=2 for v in neighbours.values()):raise ValueError('Boundary is not a collection of simple closed loops')
    remaining=set(neighbours);loops=[]
    while remaining:
        start=min(remaining);previous=None;current=start;loop=[]
        while True:
            if current in loop:raise ValueError('Repeated non-closing boundary vertex')
            loop.append(current);remaining.remove(current)
            choices=neighbours[current]-({previous} if previous is not None else set());following=min(choices)
            if following==start:break
            previous,current=current,following
        if len(loop)<3:raise ValueError('Insufficient boundary vertices')
        loops.append(loop)
    return loops

def collar_quality(rows):
    """Detect collapsed/folded correspondence before connecting retained wood."""
    rows=np.asarray(rows,float)
    along=rows[1:]-rows[:-1];around=np.roll(rows,-1,axis=1)-rows
    jacobian=np.cross(around[:-1],along);area=np.linalg.norm(jacobian,axis=2)
    reference=jacobian[0]+jacobian[-1];reference_length=np.linalg.norm(reference,axis=1)
    dot=np.sum(jacobian*reference[None,:,:],axis=2)
    return dict(minimum_quad_jacobian=float(area.min()),collapsed_quads=int((area<1e-9).sum()),reversed_quads=int((dot<0).sum()),undefined_reference_normals=int((reference_length<1e-9).sum()))
