"""Curvature-limited parallel frames for explicit tree sweeps; CPU-testable."""
import math
import numpy as np


def unit(v):
    length=np.linalg.norm(v)
    if length<1e-10:raise ValueError('Zero sweep direction')
    return v/length


def rotate(v,axis,angle):
    return v*math.cos(angle)+np.cross(axis,v)*math.sin(angle)+axis*(axis@v)*(1-math.cos(angle))


def frames(centers,radii,curvature_limit=.6):
    centers=np.asarray(centers);count=len(centers)
    desired=[unit(centers[min(count-1,j+8)]-centers[max(0,j-8)]) for j in range(count)]
    tangent=desired[0];ray=np.array([0,-math.cos(math.radians(35)),math.sin(math.radians(35))]);normal=unit(np.cross(ray,tangent));binormal=unit(np.cross(tangent,normal));result=[(normal.copy(),binormal.copy(),tangent.copy())]
    for j in range(1,count):
        step=np.linalg.norm(centers[j]-centers[j-1]);axis=np.cross(tangent,desired[j]);axislen=np.linalg.norm(axis)
        if axislen>1e-12:
            angle=min(math.acos(np.clip(tangent@desired[j],-1,1)),curvature_limit*step/max(radii[j],radii[j-1],.65));axis/=axislen
            tangent=unit(rotate(tangent,axis,angle));normal=unit(rotate(normal,axis,angle));normal=unit(normal-tangent*(normal@tangent));binormal=unit(np.cross(tangent,normal))
        result.append((normal.copy(),binormal.copy(),tangent.copy()))
    return result


def strip_orientation(vertices,sides):
    rings=np.asarray(vertices).reshape(-1,sides,3);centers=rings.mean(axis=1);bad=[];minimum=1.0;tested=0
    for j in range(len(rings)-1):
        for k in range(sides):
            l=(k+1)%sides;a,b,c,d=rings[j,k],rings[j,l],rings[j+1,l],rings[j+1,k]
            # Both triangles are checked, rather than trusting one half of a quad.
            for which,(x,y,z) in enumerate([(a,b,c),(a,c,d)]):
                normal=np.cross(y-x,z-x);mid=(x+y+z)/3;center=(centers[j]*2+centers[j+1])/3 if which==0 else (centers[j]+centers[j+1]*2)/3;radial=mid-center;den=np.linalg.norm(normal)*np.linalg.norm(radial);signed=float(normal@radial/den) if den>1e-12 else 0;minimum=min(minimum,signed);tested+=1
                if signed<=1e-7:bad.append([j,k,which,signed])
    return dict(tested=tested,nonoutward=len(bad),minimum_signed_dot=minimum,examples=bad[:20])


def curvature_limited_sweep(vertices, sides=16, sigma=1.0):
    """Test a continuous sweep; keep source-loss evidence separate from topology.

    Pixel staircase samples are smoothed by at most their measured displacement.
    Radii are reduced where a normal section would turn through its own inside
    surface. This is only a candidate: independent source coverage must decide
    whether that reduction is acceptable.
    """
    from scipy.ndimage import gaussian_filter1d
    rings = np.asarray(vertices, dtype=float).reshape(-1, sides, 3)
    original = rings.mean(axis=1)
    radii = np.linalg.norm(rings[:, 0] - original, axis=1)
    centers = gaussian_filter1d(original, sigma, axis=0, mode='nearest')
    blend = np.linspace(0, 1, len(centers))[:, None]
    centers += (original[0] - centers[0]) * (1-blend) + (original[-1] - centers[-1]) * blend
    trial_frames = frames(centers,radii,curvature_limit=.3)
    trial = np.array([center+radii[j]*(n*math.cos(a)+b*math.sin(a))
                      for j,(center,(n,b,_)) in enumerate(zip(centers,trial_frames))
                      for a in np.arange(sides)*math.tau/sides])
    trial_audit = strip_orientation(trial,sides)
    if trial_audit['nonoutward']==0:
        displacement=centers-original
        s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
        projected=np.column_stack((displacement[:,0],-s*displacement[:,1]-c*displacement[:,2]))
        trial_audit.update(max_source_center_displacement=float(np.linalg.norm(projected,axis=1).max()),max_radius_reduction=0.,radius_times_curvature_limit=None,sigma=sigma,correction='bounded frame transport without radius reduction')
        return trial,trial_audit
    # A wide bend may not admit perpendicular circular sections. Test a
    # continuous oblique section field before reducing any source-supported
    # radius. Its direction maximizes forward progress over all center steps.
    from scipy.optimize import minimize
    directions=np.diff(centers,axis=0)
    directions/=np.linalg.norm(directions,axis=1)[:,None]
    initial=unit(centers[-1]-centers[0])
    solution=minimize(lambda x:-x[3],np.r_[initial,np.min(directions@initial)],
                      constraints=[{'type':'ineq','fun':lambda x:1-x[:3]@x[:3]},
                                   {'type':'ineq','fun':lambda x:directions@x[:3]-x[3]}],
                      method='SLSQP',options={'ftol':1e-10,'maxiter':200})
    if solution.success and solution.x[3]>.01:
        axis=unit(solution.x[:3])
        ray=np.array([0,-math.cos(math.radians(35)),math.sin(math.radians(35))])
        normal=unit(np.cross(ray,axis));binormal=unit(np.cross(axis,normal))
        oblique=np.array([center+radii[j]*(normal*math.cos(a)+binormal*math.sin(a))
                          for j,center in enumerate(centers) for a in np.arange(sides)*math.tau/sides])
        check=strip_orientation(oblique,sides)
        if check['nonoutward']==0:
            displacement=centers-original;s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
            projected=np.column_stack((displacement[:,0],-s*displacement[:,1]-c*displacement[:,2]))
            check.update(max_source_center_displacement=float(np.linalg.norm(projected,axis=1).max()),max_radius_reduction=0.,sigma=sigma,correction='oblique parallel sections; actual shape review required',minimum_axis_step_dot=float(np.min(directions@axis)))
            return oblique,check
    tangents = [unit(centers[min(len(centers)-1,j+1)]-centers[max(0,j-1)]) for j in range(len(centers))]
    ray = np.array([0,-math.cos(math.radians(35)),math.sin(math.radians(35))])
    normal = unit(np.cross(ray,tangents[0]))
    transported = [(normal.copy(),unit(np.cross(tangents[0],normal)))]
    for before,after in zip(tangents,tangents[1:]):
        axis = np.cross(before,after)
        length = np.linalg.norm(axis)
        if length > 1e-12:
            normal = rotate(normal,axis/length,math.atan2(length,float(before@after)))
        elif before@after < 0:
            raise ValueError('Reversing centerline cannot form a continuous sweep')
        normal = unit(normal-after*(normal@after))
        transported.append((normal.copy(),unit(np.cross(after,normal))))
    steps = np.linalg.norm(np.diff(centers,axis=0),axis=1)
    turns = np.linalg.norm(np.diff(tangents,axis=0),axis=1)
    bounds = .45*steps/np.maximum(turns,1e-8)
    limited = np.minimum(radii,np.minimum(np.r_[bounds[0],bounds],np.r_[bounds,bounds[-1]]))
    for order in (range(1,len(limited)),range(len(limited)-2,-1,-1)):
        for j in order:
            k=j-1 if order.step>0 else j+1
            limited[j]=min(limited[j],limited[k]+.16*np.linalg.norm(centers[j]-centers[k]))
    result = np.array([center+limited[j]*(n*math.cos(a)+b*math.sin(a))
                       for j,(center,(n,b)) in enumerate(zip(centers,transported))
                       for a in np.arange(sides)*math.tau/sides])
    audit = strip_orientation(result,sides)
    displacement = centers-original
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
    projected = np.column_stack((displacement[:,0],-s*displacement[:,1]-c*displacement[:,2]))
    audit.update(max_source_center_displacement=float(np.linalg.norm(projected,axis=1).max()),
                 max_radius_reduction=float(np.max(radii-limited)),
                 radius_times_curvature_limit=.45, sigma=sigma)
    return result,audit
