"""Reconstruct the observed shallow recess in the Hall's south dormer gable."""
import bpy
from mathutils import Vector
from derby_round2_east_hall import _replace


def _half_mesh(left, right, eave, window, depth=1.0):
    """Closed half-prism with a gridded end face and an inset window panel.

    Matching front/back grids preserve manifold edges where the rectangular
    recess crosses the shared ridge plane. The long rear dormer is retained.
    """
    ridge, bottom, length = 444.82, 318.76, 171.95
    center = Vector((1378.92, -2257.32, 0))
    across = Vector((.6560574, -.7547109, 0)).normalized()
    backward = Vector((.7547109, .6560574, 0)).normalized()
    outer = left if left < 0 else right
    slope = (eave-ridge)/outer
    roof = lambda u: ridge+slope*u
    wl, wr, wb, wt = window
    us = {left, right}
    us.update(u for u in (wl,wr) if left < u < right)
    zs = [bottom, wb, wt, ridge]
    us.update((z-ridge)/slope for z in zs if left < (z-ridge)/slope < right)
    us = sorted(us)
    cells=[]
    def clip(points):
        result=[]
        for p,q in zip(points,points[1:]+points[:1]):
            fp,fq=p[1]-roof(p[0]),q[1]-roof(q[0])
            if fp<=1e-8:result.append(p)
            if (fp<-1e-8 and fq>1e-8) or (fp>1e-8 and fq<-1e-8):
                t=fp/(fp-fq);result.append((p[0]+t*(q[0]-p[0]),p[1]+t*(q[1]-p[1])))
        clean=[]
        for p in result:
            p=tuple(round(v,7) for v in p)
            if not clean or p!=clean[-1]:clean.append(p)
        if len(clean)>1 and clean[0]==clean[-1]:clean.pop()
        return clean
    for ua,ub in zip(us,us[1:]):
        for za,zb in zip(zs,zs[1:]):
            polygon=clip([(ua,za),(ub,za),(ub,zb),(ua,zb)])
            if len(polygon)<3:continue
            area=abs(sum(p[0]*q[1]-q[0]*p[1] for p,q in zip(polygon,polygon[1:]+polygon[:1])))/2
            if area<1e-7:continue
            inset=depth if wl-1e-7<=ua and ub<=wr+1e-7 and wb-1e-7<=za and zb<=wt+1e-7 else 0.
            cells.append((polygon,inset))
    vertices,faces,lookup=[],[],{}
    def vertex(p,d):
        key=(round(p[0],7),round(p[1],7),round(d,7))
        if key not in lookup:
            lookup[key]=len(vertices)
            vertices.append(tuple(center+across*p[0]+backward*d+Vector((0,0,p[1]))))
        return lookup[key]
    edges={}
    for polygon,d in cells:
        faces.append(tuple(vertex(p,d) for p in polygon))
        faces.append(tuple(vertex(p,length) for p in reversed(polygon)))
        for p,q in zip(polygon,polygon[1:]+polygon[:1]):
            edges.setdefault(tuple(sorted((p,q))),[]).append((p,q,d))
    for sides in edges.values():
        if len(sides)==1:
            p,q,d=sides[0];faces.append((vertex(p,d),vertex(q,d),vertex(q,length),vertex(p,length)))
        elif len(sides)==2:
            p,q,d=sides[0];other=sides[1][2]
            if abs(d-other)>1e-7:faces.append((vertex(p,d),vertex(q,d),vertex(q,other),vertex(p,other)))
        else:raise ValueError('Unexpected dormer grid adjacency')
    # The shared ridge side changes its front endpoint at the recess lip.
    # Insert those collinear endpoints in the adjoining long side edges.
    points=[Vector(p) for p in vertices]
    stitched=[]
    for face in faces:
        polygon=[]
        for a,b in zip(face,face[1:]+face[:1]):
            polygon.append(a);delta=points[b]-points[a]
            cuts=[]
            for i,p in enumerate(points):
                if i in (a,b):continue
                t=(p-points[a]).dot(delta)/delta.length_squared
                if 1e-6<t<1-1e-6 and (points[a]+t*delta-p).length<.0002:cuts.append((t,i))
            polygon.extend(i for t,i in sorted(cuts))
        stitched.append(tuple(polygon))
    return vertices,stitched


def refine_south_dormer_window():
    """Retain both dormer ends, adding only the source-observed front recess.

    The dark front window is about source x1377..1383,y949..960. Its modest depth
    is inferred; the artwork provides width/height/placement, not hidden depth.
    The two half-prisms previously had unwelded/open bottoms and slightly offset
    ridge coordinates. Their measured outer profile is retained within0.5 units.
    """
    changes=[]
    window=(-3.5,6.0,412.5,420.0)
    for node,left,right,eave in ((202,0.,19.35,408.96),(204,-19.35,0.,417.5)):
        found=[o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type=='MESH' and not o.hide_render and o.get('source_node')==f'building-{node:03d}']
        if len(found)!=1:raise ValueError(f'Expected one visible dormer half{node}')
        obj=found[0];tag='south-dormer-observed-window-v2'
        if obj.get('east_hall_dormer_window')==tag:
            changes.append({'source_node':obj['source_node'],'status':'already-refined'});continue
        vertices,faces=_half_mesh(left,right,eave,window)
        result=_replace(obj,vertices,faces)
        obj['east_hall_dormer_window']=tag
        obj['east_hall_dormer_window_depth_inferred']=1.0
        result.update({'window_local_bounds':window,'window_depth':1.0,'depth_inferred':True,'rear_end_retained':True})
        changes.append(result)
    return changes
