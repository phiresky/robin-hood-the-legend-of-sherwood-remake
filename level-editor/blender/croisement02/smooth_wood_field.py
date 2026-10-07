"""Continuous local field rim; binary pixel corners are not physical vertices."""
import numpy as np
from scipy.ndimage import gaussian_filter, map_coordinates, distance_transform_edt
from continuous_wood_field import SIN, COS, RAY, center_depth


def smooth_shell(body, thickness, origin, ground, sigma=.8, step=.5):
    """Clip triangles against a continuous positive thickness field.

The threshold retains all observed body pixel centers. Front and back meet on
the interpolated zero contour, rather than following each binary pixel edge.
This function does not read or modify the retained trunk.
"""
    padding = 4
    potential = gaussian_filter(np.pad(thickness**2/2, padding), sigma)
    padded_body = np.pad(body, padding)
    distance = distance_transform_edt(padded_body)-distance_transform_edt(~padded_body)
    field = gaussian_filter(distance, sigma)
    interior = field[padding:-padding,padding:-padding][body]
    bias = max(0., .1-float(interior.min()))
    field += bias
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
    radii = np.sqrt(2*np.maximum(0,sampled_potential)*(-np.expm1(-np.maximum(0,scalar)/.7)))
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
    return np.asarray(vertices),np.asarray(faces,int),report
