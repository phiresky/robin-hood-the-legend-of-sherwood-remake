"""Continuous local field rim; binary pixel corners are not physical vertices."""
import numpy as np
from scipy.ndimage import gaussian_filter, map_coordinates, distance_transform_edt
from continuous_wood_field import SIN, COS, RAY, center_depth


def constrained_rim(body, weight=32., padding=8):
    """CPU experiment: smooth curvature with local observed-center constraints.

    Unlike a global threshold shift, this only forces the field outward where
    an observed center requires it. This is not yet the model construction
    default: mesh/collar and saved appearance guards must still pass.
    """
    from scipy.ndimage import laplace
    from scipy.optimize import minimize
    padded = np.pad(body, padding)
    target = distance_transform_edt(padded)-distance_transform_edt(~padded)
    lower = np.full(target.shape, -np.inf)
    lower[padded] = .1

    def objective(flat):
        field = flat.reshape(target.shape)
        delta = field-target
        curvature = laplace(field, mode='reflect')
        energy = .5*(np.sum(delta*delta)+weight*np.sum(curvature*curvature))
        gradient = delta+weight*laplace(curvature, mode='reflect')
        return energy, gradient.ravel()

    initial = np.maximum(gaussian_filter(target, 1.6), lower)
    result = minimize(objective, initial.ravel(), jac=True, method='L-BFGS-B',
                      bounds=list(zip(lower.ravel(), np.full(target.size, np.inf))),
                      options={'maxiter': 300, 'ftol': 1e-10, 'gtol': 1e-5, 'maxcor': 8})
    if not result.success:
        raise ValueError('Constrained rim did not converge: '+str(result.message))
    field = result.x.reshape(target.shape)
    if np.any(field[padded] < .1-1e-8):
        raise ValueError('Constrained rim lost an observed center')
    return field, dict(weight=weight, padding=padding, iterations=result.nit,
                       method='Local observed-center bounds with quadratic Laplacian regularization',
                       model_validated=False)


def smooth_shell(body, thickness, origin, ground, sigma=.8, step=.5, local_rim_weight=None, depth_scale=1.):
    """Clip triangles against a continuous positive thickness field.

The threshold retains all observed body pixel centers. Front and back meet on
the interpolated zero contour, rather than following each binary pixel edge.
This function does not read or modify the retained trunk.
"""
    if not np.isfinite(depth_scale) or depth_scale <= 0:
        raise ValueError('Inferred depth scale must be positive and finite')
    padding = 8 if local_rim_weight is not None else 4
    potential = gaussian_filter(np.pad(thickness**2/2, padding), sigma)
    padded_body = np.pad(body, padding)
    distance = distance_transform_edt(padded_body)-distance_transform_edt(~padded_body)
    if local_rim_weight is None:
        field = gaussian_filter(distance, sigma)
        interior = field[padding:-padding,padding:-padding][body]
        bias = max(0., .1-float(interior.min()))
        field += bias
        constrained_report = None
    else:
        field, constrained_report = constrained_rim(body, local_rim_weight, padding)
        interior = field[padding:-padding,padding:-padding][body]
        bias = 0.
    h, w = body.shape
    ys = np.arange(-padding, h+padding+step/2, step)
    xs = np.arange(-padding, w+padding+step/2, step)
    yy, xx = np.meshgrid(ys, xs, indexing='ij')
    values = map_coordinates(field, [yy+padding, xx+padding], order=3, mode='constant', cval=-padding)
    # Body samples are at source-pixel centers; geometry may cover a fraction
    # beyond the mask, but the unchanged source ownership controls its texture.
    coordinates = np.column_stack((xx.ravel()+origin[0]+.5, yy.ravel()+origin[1]+.5))
    scalar = values.ravel()
    sampled_potential = map_coordinates(potential,[yy+padding,xx+padding],order=3,mode='constant',cval=0).ravel()
    radii = depth_scale*np.sqrt(2*np.maximum(0,sampled_potential)*(-np.expm1(-np.maximum(0,scalar)/.7)))
    source_ids = {}; intersections = {}; points2 = []; offsets = []; faces2 = []

    def original(i):
        i = int(i)
        if i not in source_ids:
            source_ids[i] = len(points2); points2.append(coordinates[i]); offsets.append(radii[i])
        return source_ids[i]

    def crossing(a, b):
        key = tuple(sorted((int(a), int(b))))
        if key not in intersections:
            fraction = scalar[a]/(scalar[a]-scalar[b])
            intersections[key] = len(points2)
            points2.append(coordinates[a]+fraction*(coordinates[b]-coordinates[a])); offsets.append(0.)
        return intersections[key]

    stride = len(xs)
    active = (values[:-1,:-1]>0)|(values[1:,:-1]>0)|(values[:-1,1:]>0)|(values[1:,1:]>0)
    for y,x in zip(*np.where(active)):
        a=y*stride+x; b=a+1; c=a+stride; d=c+1
        triangles = [(a,b,d),(a,d,c)] if (x+y)%2 == 0 else [(a,b,c),(b,d,c)]
        for triangle in triangles:
            polygon = []
            for i,j in zip(triangle, np.roll(triangle,-1)):
                if scalar[i] > 0: polygon.append(original(i))
                if (scalar[i]>0) != (scalar[j]>0): polygon.append(crossing(i,j))
            for i in range(1,len(polygon)-1): faces2.append((polygon[0],polygon[i],polygon[i+1]))
    points2 = np.asarray(points2); offsets = np.asarray(offsets)
    base = np.column_stack((points2[:,0],-points2[:,1]*SIN,-points2[:,1]*COS))+center_depth(points2[:,1],ground)[:,None]*RAY
    vertices = list(base+offsets[:,None]*RAY)
    rear = np.arange(len(base))
    for i in np.where(offsets>0)[0]:
        rear[i]=len(vertices);vertices.append(base[i]-offsets[i]*RAY)
    faces=[]
    for a,b,c in faces2:
        faces.append((c,b,a));faces.append((int(rear[a]),int(rear[b]),int(rear[c])))
    lattice_positive = field[padding:-padding,padding:-padding]>0
    if np.any(body & ~lattice_positive): raise ValueError('Continuous rim excludes observed pixel centers')
    report=dict(method='Smoothed signed-distance rim with independent Poisson thickness and interpolated zero contour',sigma=sigma,grid_step=step,conservative_rim_bias=bias,minimum_observed_signed_distance=float(interior.min()+bias),observed_centers_preserved=int(body.sum()),missing_observed_centers=0,pixel_center_silhouette_iou=float((body&lattice_positive).sum()/(body|lattice_positive).sum()),extra_center_pixels=int((lattice_positive&~body).sum()),limits=['A center-coverage guard is not a rasterized pixel-area/terrain proof.','Source mask is unchanged; additional inferred silhouette margin requires visual review.'])
    if constrained_report is not None:
        report['method'] = 'Locally constrained curvature rim with independent Poisson thickness'
        report['constrained_rim'] = constrained_report
    report['inferred_depth_scale'] = depth_scale
    return np.asarray(vertices),np.asarray(faces,int),report
