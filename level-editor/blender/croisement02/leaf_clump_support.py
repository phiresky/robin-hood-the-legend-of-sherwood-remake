"""Exact affine native-ray clearance between projected triangle interiors."""
import numpy as np
from leaf_receiver_visibility import clip_halfplane,area

def triangle(points,ray,sine,cosine):
    p=np.asarray(points,dtype=float);xy=np.column_stack((p[:,0],-p[:,1]*sine-p[:,2]*cosine));matrix=np.column_stack((xy,np.ones(3)))
    if abs(np.linalg.det(matrix))<1e-8:return None
    return dict(xy=xy,depth=np.linalg.solve(matrix,p@np.asarray(ray)),minimum=xy.min(axis=0),maximum=xy.max(axis=0))

def overlap(a,b):
    poly=list(a);signed=sum(float(x[0]*y[1]-y[0]*x[1])for x,y in zip(b,np.roll(b,-1,axis=0)));orientation=1 if signed>0 else-1
    for x,y in zip(b,np.roll(b,-1,axis=0)):
        d=y-x;c=orientation*np.array([-d[1],d[0],d[1]*x[0]-d[0]*x[1]]);poly=clip_halfplane(poly,c)
        if area(poly)<1e-9:return []
    return poly

def clearance(body,receivers,margin=.02):
    """Maximum receiver minus body depth over exact shared triangle domains."""
    minimum=np.array([r['minimum']for r in receivers]);maximum=np.array([r['maximum']for r in receivers]);required=0.;witness=None;pairs=0
    for b in body:
        ids=np.flatnonzero(np.all(maximum>=b['minimum']-1e-8,axis=1)&np.all(minimum<=b['maximum']+1e-8,axis=1))
        for i in ids:
            r=receivers[int(i)];poly=overlap(b['xy'],r['xy'])
            if not poly:continue
            pairs+=1
            for p in poly:
                q=np.array([*p,1.]);bd=float(q@b['depth']);rd=float(q@r['depth']);need=rd-bd+margin
                if need>required:required=need;witness=dict(screen=p.tolist(),body_depth=bd,receiver_depth=rd,receiver=int(i),clearance_after=margin)
    if not pairs:raise ValueError('No supporting receiver overlaps this closed clump')
    return required,witness,pairs
