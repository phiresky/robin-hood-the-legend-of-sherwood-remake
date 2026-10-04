"""Solve a foliage object's visible lower extent against the audited bank."""
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from catalog import scenery_workspace
from evidence_io import sha
from opacity_bounds import measure
from tree_geometry import RAY,SIN,COS

def load_support():
    bank=scenery_workspace('croisement02-north-woodland-bank');digest=sha(bank/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(bank/'model.blend'))
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==bank.name]
    if sorted(o.get('source_node') for o in objects)!=[f'building-{i:03}' for i in range(5)]:raise ValueError('Unexpected bank part set')
    support=[(o.get('source_node'),BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons])) for o in objects]
    return bank,digest,support

def visible_points(obj):
    """Sample actual saved UV alpha in world coordinates."""
    mesh=obj.data;mesh.calc_loop_triangles();uv=mesh.uv_layers['Foliage UV'];matrix=np.asarray(obj.matrix_world)
    world=np.asarray([(*v.co,1) for v in mesh.vertices])@matrix.T;images={};cloud=[]
    for tri in mesh.loop_triangles:
        mat=mesh.materials[tri.material_index];image=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        if image.name not in images:
            w,h=image.size;images[image.name]=np.asarray(image.pixels[:],dtype=np.float32).reshape(h,w,4)[:,:,3]
        alpha=images[image.name];h,w=alpha.shape;coords=np.asarray([uv.data[i].uv for i in tri.loops])*[w,h]
        x0,y0=np.floor(coords.min(0)).astype(int);x1,y1=np.ceil(coords.max(0)).astype(int);x0,y0=max(0,x0),max(0,y0);x1,y1=min(w,x1),min(h,y1)
        if x1<=x0 or y1<=y0:continue
        yy,xx=np.mgrid[y0:y1:2,x0:x1:2];q=np.column_stack((xx.ravel()+.5,yy.ravel()+.5));a,b,c=coords;basis=np.column_stack((b-a,c-a))
        if abs(np.linalg.det(basis))<1e-9:continue
        bc=(q-a)@np.linalg.inv(basis).T;inside=(bc[:,0]>=0)&(bc[:,1]>=0)&(bc.sum(1)<=1)&(alpha[yy.ravel(),xx.ravel()]>.5)
        if inside.any():cloud.append(np.column_stack((1-bc[inside].sum(1),bc[inside]))@world[list(tri.vertices),:3])
    if not cloud:raise ValueError('No visible foliage support samples')
    return np.concatenate(cloud)

def lower_anchor(obj):
    points=visible_points(obj);minimum=points[:,2].min();low=points[points[:,2]<=minimum+2.]
    anchor=np.median(low,axis=0);anchor[2]=minimum
    return anchor,int(len(low))

def place(obj,authority):
    bank,digest,support=authority;bounds=measure(obj);lo=np.array(bounds['bounds_min']);hi=np.array(bounds['bounds_max']);center,sample_count=lower_anchor(obj)
    def surface(distance):
        x,y=center[0],center[1]-distance*COS;hits=[]
        for name,bvh in support:
            p,normal,face,d=bvh.ray_cast(Vector((float(x),float(y),1000)),Vector((0,0,-1)))
            if p is not None:hits.append((p.z,name))
        return max(hits) if hits else (0.,'ground-datum-outside-bank')
    def residual(distance):return lo[2]+distance*SIN-surface(distance)[0]-.5
    lower,upper=-200.,200.
    if residual(lower)>0 or residual(upper)<0:raise ValueError('Cannot bracket support')
    for _ in range(55):
        mid=(lower+upper)/2
        if residual(mid)<0:lower=mid
        else:upper=mid
    distance=(lower+upper)/2
    if abs(residual(distance))>.02:raise ValueError(f'Support discontinuity for {obj.name}, lower anchor{center.tolist()}, distance{distance}, residual{residual(distance)}, surface{surface(distance)}')
    delta=RAY*distance
    for v in obj.data.vertices:v.co+=delta
    obj.data.update();after=measure(obj);height,part=surface(distance)
    if sha(bank/'model.blend')!=digest:raise ValueError('Bank changed during support solve')
    return dict(bank_worker=str(bank),bank_model_sha256=digest,previous_opacity_bounds=bounds,current_opacity_bounds=after,world_delta=list(delta),support_part=part,support_point=[float(center[0]),float(center[1]-distance*COS),height],clearance=.5,lower_visible_anchor_samples=sample_count,anchor_method='Median XY of actual opaque samples within2 units of minimumZ',projection_delta=[float(delta.x),float(-delta.y*SIN-delta.z*COS)],limitation='Lower visible fringe support is a placement hypothesis; slope intersections and native occlusion need actual joint review.')
