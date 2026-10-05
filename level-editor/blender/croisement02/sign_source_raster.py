"""Double-precision native first-hit raster without transparent ray-step epsilon."""
import numpy as np
from tree_geometry import SIN,COS,RAY


def raster(obj,box,with_details=False):
    mesh=obj.data;mesh.calc_loop_triangles();uv=mesh.uv_layers['Foliage UV'];ownership=mesh.color_attributes.get('Source ownership')
    world=np.array([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);projection=np.column_stack((world[:,0],-SIN*world[:,1]-COS*world[:,2]));ray=np.array(RAY)
    width,height=box[2]-box[0],box[3]-box[1];depth=np.full((height,width),-np.inf);rgba=np.zeros((height,width,4),dtype=np.float32);roles=np.zeros((height,width),dtype=np.uint8);faces=np.full((height,width),-1,dtype=np.int32);images={}
    for tri in mesh.loop_triangles:
        mat=mesh.materials[tri.material_index];points=world[list(tri.vertices)]
        if mat.node_tree.nodes.get('One-sided foliage') and np.cross(points[1]-points[0],points[2]-points[0])@ray<=0:continue
        coords=projection[list(tri.vertices)];a,b,c=coords;basis=np.column_stack((b-a,c-a));det=np.linalg.det(basis)
        if abs(det)<1e-10:continue
        lo=np.maximum(box[:2],np.floor(coords.min(0)).astype(int));hi=np.minimum(box[2:],np.ceil(coords.max(0)).astype(int))
        if np.any(hi<=lo):continue
        yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];q=np.column_stack((xx.ravel()+.5,yy.ravel()+.5));bc=(q-a)@np.linalg.inv(basis).T
        inside=(bc[:,0]>=-1e-8)&(bc[:,1]>=-1e-8)&(bc.sum(1)<=1+1e-8)
        if not inside.any():continue
        bary=np.column_stack((1-bc[inside].sum(1),bc[inside]));px=xx.ravel()[inside]-box[0];py=yy.ravel()[inside]-box[1]
        im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);w,h=im.size
        if im.name not in images:images[im.name]=np.array(im.pixels[:],dtype=np.float32).reshape(h,w,4)
        pixels=images[im.name];tuv=bary@np.array([tuple(uv.data[l].uv) for l in tri.loops]);tex=np.floor(tuv*[w,h]).astype(int);color=pixels[tex[:,1]%h,tex[:,0]%w];z=(bary@points)@ray
        take=(color[:,3]>.5)&(z>depth[py,px]);depth[py[take],px[take]]=z[take];rgba[py[take],px[take]]=color[take]
        known=bool(mat.get('foliage_observed')) or bool(ownership and max(ownership.data[i].color[0] for i in tri.loops)>.5)
        roles[py[take],px[take]]=1 if known else 2
        faces[py[take],px[take]]=tri.polygon_index
    return (rgba,roles,depth,faces) if with_details else (rgba,roles)
