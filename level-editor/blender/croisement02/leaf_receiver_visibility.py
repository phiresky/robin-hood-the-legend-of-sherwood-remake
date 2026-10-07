"""Clip planar receiver polygons to their frontmost native-ray domains."""
import numpy as np

def clip_halfplane(poly,coefficient,epsilon=0.):
    if not poly:return []
    out=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        da=float(np.dot(coefficient[:2],a)+coefficient[2])-epsilon
        db=float(np.dot(coefficient[:2],b)+coefficient[2])-epsilon
        if da>=0:out.append(a)
        if (da>=0)!=(db>=0):out.append(a+(b-a)*(da/(da-db)))
    return out

def area(poly):
    return abs(sum(float(a[0]*b[1]-b[0]*a[1])for a,b in zip(poly,poly[1:]+poly[:1])))/2 if len(poly)>=3 else 0.

def subtract_convex(subject,cutter):
    if len(cutter)<3:return [subject]
    signed=sum(float(a[0]*b[1]-b[0]*a[1])for a,b in zip(cutter,cutter[1:]+cutter[:1]));orientation=1 if signed>0 else -1
    inside=subject;outside=[]
    for a,b in zip(cutter,cutter[1:]+cutter[:1]):
        d=b-a;c=orientation*np.array([-d[1],d[0],d[1]*a[0]-d[0]*a[1]])
        fragment=clip_halfplane(inside,-c)
        if area(fragment)>1e-9:outside.append(fragment)
        inside=clip_halfplane(inside,c)
        if area(inside)<=1e-9:break
    return outside

def frontmost(candidates,ray):
    """Input polygons have screen xy, affine world mapping, and owning surface."""
    rows=[]
    for row in candidates:
        depth=row['mapping']@np.array(ray);pieces=[row['polygon']]
        for other in candidates:
            if other is row:continue
            difference=other['mapping']@np.array(ray)-depth
            occluder=clip_halfplane(other['polygon'],difference,1e-6)
            if area(occluder)<=1e-9:continue
            pieces=[fragment for piece in pieces for fragment in subtract_convex(piece,occluder)]
            if not pieces:break
        for polygon in pieces:
            if area(polygon)>1e-8:rows.append(dict(owner=row['owner'],points=[np.array([*p,1.])@row['mapping']for p in polygon]))
    return rows
