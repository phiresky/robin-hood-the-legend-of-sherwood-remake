"""Measure visible crown bounds by sampling actual UV alpha on saved triangles."""
import numpy as np


def measure(obj):
    mesh=obj.data;mesh.calc_loop_triangles();uv=mesh.uv_layers['Foliage UV']
    matrix=np.asarray(obj.matrix_world);world=np.asarray([(*v.co,1) for v in mesh.vertices])@matrix.T
    images={};low=np.full(3,np.inf);high=np.full(3,-np.inf);samples=0
    for tri in mesh.loop_triangles:
        mat=mesh.materials[tri.material_index]
        image=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        if image.name not in images:
            w,h=image.size;images[image.name]=np.asarray(image.pixels[:],dtype=np.float32).reshape(h,w,4)[:,:,3]
        alpha=images[image.name];h,w=alpha.shape
        coords=np.asarray([uv.data[i].uv for i in tri.loops])*[w,h]
        x0,y0=np.floor(coords.min(axis=0)).astype(int);x1,y1=np.ceil(coords.max(axis=0)).astype(int)
        x0,y0=max(0,x0),max(0,y0);x1,y1=min(w,x1),min(h,y1)
        if x1<=x0 or y1<=y0:continue
        yy,xx=np.mgrid[y0:y1:2,x0:x1:2];q=np.column_stack((xx.ravel()+.5,yy.ravel()+.5))
        a,b,c=coords;basis=np.column_stack((b-a,c-a));det=np.linalg.det(basis)
        if abs(det)<1e-9:continue
        bc=(q-a)@np.linalg.inv(basis).T;inside=(bc[:,0]>=0)&(bc[:,1]>=0)&(bc.sum(axis=1)<=1)
        inside &=alpha[yy.ravel(),xx.ravel()]>.5
        if not inside.any():continue
        bary=np.column_stack((1-bc[inside].sum(axis=1),bc[inside]));points=bary@world[list(tri.vertices),:3]
        low=np.minimum(low,points.min(axis=0));high=np.maximum(high,points.max(axis=0));samples+=len(points)
    if not samples:raise ValueError('No opaque crown samples')
    size=high-low
    return dict(bounds_min=low.tolist(),bounds_max=high.tolist(),width=float(size[0]),depth=float(size[1]),depth_width_ratio=float(size[1]/size[0]),opaque_samples=samples,method='Actual saved UV alpha > 0.5, triangle barycentrics, two-texel spacing; approximate raster bounds')
