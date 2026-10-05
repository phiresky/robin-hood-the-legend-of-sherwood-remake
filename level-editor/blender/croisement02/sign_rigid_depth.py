"""Translate complete paired leaf fragments against sampled opaque solid clearance."""
import json
from collections import defaultdict
from pathlib import Path
import numpy as np
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from scipy.spatial import cKDTree
from tree_geometry import SIN,COS,RAY
from catalog import OUT
from evidence_io import sha
from sign_context_import import append_verified


def groups_for(mesh,world,projection):
    parent=list(range(len(world)))
    def root(i):
        while parent[i]!=i:parent[i]=parent[parent[i]];i=parent[i]
        return i
    def union(a,b):parent[root(b)]=root(a)
    for p in mesh.polygons:
        for i in p.vertices[1:]:union(p.vertices[0],i)
    explicit=mesh.attributes.get('Sign leaf pair')
    if explicit:
        anchors={}
        for p in mesh.polygons:
            identity=explicit.data[p.index].value
            if identity in anchors:union(anchors[identity],p.vertices[0])
            else:anchors[identity]=p.vertices[0]
    else:
        for a,b in cKDTree(world).query_pairs(.031):
            if np.max(abs(projection[a]-projection[b]))<.001:union(a,b)
    groups=defaultdict(list)
    for i in range(len(world)):groups[root(i)].append(i)
    return groups,[root(i) for i in range(len(world))]


def bend(obj,detail):
    mesh=obj.data;mesh.calc_loop_triangles();world=np.array([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);ray=np.array(RAY)
    projection=np.column_stack((world[:,0],-SIN*world[:,1]-COS*world[:,2]))
    groups,roots=groups_for(mesh,world,projection)
    def root(i):return roots[i]
    needs=defaultdict(float);source_constraints={}
    for row in detail['pixels']:
        for hit in row.get('blockers',[]):
            polygon=mesh.polygons[hit['polygon']];g=root(polygon.vertices[0]);needs[g]=max(needs[g],hit['required_retreat']+.25)
            key=tuple(row['source_pixel']);source_constraints[key]=max(source_constraints.get(key,0),hit['required_retreat']+.25)
    sourcepoints=np.array(list(source_constraints))+.5;demands=np.array(list(source_constraints.values()))
    manifest=json.loads((OUT/'restart2-fence/sign-neighbors-v4/manifest.json').read_text());trees=[];hashes={}
    transform_path=OUT/'restart2-fence/sign-context-evaluated-transforms-v1.json';transform_reference=json.loads(transform_path.read_text())['inputs'];transform_receipts={}
    for key in ['north-woodland-bank','west-rock-outcrop']:
        info=manifest['inputs'][key];path=Path(info['worker'])/'model.blend';assert sha(path)==info['model_sha256'];hashes[key]=sha(path)
        assert transform_reference[key]['model_sha256']==info['model_sha256']
        imported,transform_receipts[key]=append_verified(bpy.context.scene,path,info['objects'],transform_reference[key]['objects'])
        verts=[];tris=[]
        for o in imported:
            o.data.calc_loop_triangles();offset=len(verts);verts.extend([tuple(o.matrix_world@v.co) for v in o.data.vertices]);tris.extend([tuple(offset+i for i in t.vertices) for t in o.data.loop_triangles])
        trees.append(BVHTree.FromPolygons(verts,tris,all_triangles=True))
        for o in imported:bpy.data.objects.remove(o,do_unlink=True)
    desired={};limits={};samples=defaultdict(int)
    for g,indices in groups.items():
        center=projection[indices].mean(0);distance=np.linalg.norm(sourcepoints-center,axis=1)
        field=float(np.max(demands*np.exp(-(np.maximum(distance-4,0)/20)**2)))
        desired[g]=max(needs[g],field*np.clip((float(world[indices,2].mean())-.5)/80,0,1))
        limits[g]=max(0,(float(world[indices,2].min())-.5)/SIN)
    def constrain(g,points):
        if desired[g]<1e-5:return
        for point in points:
            limits[g]=min(limits[g],max(0,(float(point[2])-.5)/SIN))
            for tree in trees:
                hit,normal,index,distance=tree.ray_cast(Vector(point),-RAY,10000)
                if hit is None:continue
                # An exiting first hit identifies an existing interior point; keep it fixed.
                allowed=max(0,distance-.25) if normal.dot(-RAY)<0 else 0
                limits[g]=min(limits[g],allowed)
            samples[g]+=1
    for g,indices in groups.items():constrain(g,world[indices])
    images={};uv=mesh.uv_layers['Foliage UV']
    def clip(poly,axis,bound,keep_greater):
        out=[]
        for a,b in zip(poly,poly[1:]+poly[:1]):
            va=(a[axis]-bound)*(1 if keep_greater else -1);vb=(b[axis]-bound)*(1 if keep_greater else -1)
            if va>=0:out.append(a)
            if (va<0<=vb) or (vb<0<=va):out.append(a+(b-a)*(va/(va-vb)))
        return out
    for tri in mesh.loop_triangles:
        g=root(tri.vertices[0])
        if desired[g]<1e-5 or limits[g]==0:continue
        mat=mesh.materials[tri.material_index];im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);w,h=im.size
        if im.name not in images:images[im.name]=np.array(im.pixels[:],dtype=np.float32).reshape(h,w,4)[:,:,3]
        alpha=images[im.name];coords=np.array([uv.data[i].uv for i in tri.loops])*[w,h];a,b,c=coords;basis=np.column_stack((b-a,c-a))
        if abs(np.linalg.det(basis))<1e-9:continue
        inverse=np.linalg.inv(basis);lo=np.maximum(0,np.floor(coords.min(0)).astype(int));hi=np.minimum([w,h],np.ceil(coords.max(0)).astype(int))
        for y in range(lo[1],hi[1]):
            for x in range(lo[0],hi[0]):
                if alpha[y,x]<=.5:continue
                poly=list(coords.copy())
                for axis,bound,greater in [(0,x,True),(0,x+1,False),(1,y,True),(1,y+1,False)]:
                    if len(poly)<3:break
                    poly=clip(poly,axis,bound,greater)
                if len(poly)<3:continue
                q=np.array(poly);q=np.vstack((q,q.mean(0)));bc=(q-a)@inverse.T;bary=np.column_stack((1-bc.sum(1),bc));constrain(g,bary@world[list(tri.vertices)])
    changes=[];inverse=obj.matrix_world.inverted()
    for g,indices in groups.items():
        shift=min(desired[g],limits[g]);delta=-RAY*shift
        for i in indices:mesh.vertices[i].co=inverse@(Vector(world[i])+delta)
        if shift>1e-5 or needs[g]>0:changes.append(dict(component=g,vertices=len(indices),requested=desired[g],required_for_sampled_sign=needs[g],clearance_limit=limits[g],applied=shift,opaque_footprint_samples=samples[g],unresolved=shift+1e-5<needs[g]))
    mesh.update()
    report=dict(method='Rigid paired fragments follow one smooth source-screen envelope, capped by ground and bank/rock ray clearance over opaque UV texel footprint vertices and centroids. Original front/back spacing and UV are unchanged.',paired_components=len(groups),changed_components=sum(c['applied']>1e-5 for c in changes),required_components=len([c for c in changes if c['required_for_sampled_sign']>0]),unresolved_required_components=sum(c['unresolved'] for c in changes),maximum_shift=max(c['applied'] for c in changes),components=changes,bank_inputs=hashes,context_transform_receipt_sha256=sha(transform_path),verified_context_transforms=transform_receipts,limitations=['Opaque footprint sampling includes every clipped texel cell corner and centroid; bank projection breaklines inside a cell still require reopened crossing verification.','Preexisting bank-interior fragments stay fixed.','Rigid neighboring fragments approximate a smooth envelope; actual oblique review must check gaps.'])

    return json.loads(json.dumps(report,default=lambda v:v.item()))
