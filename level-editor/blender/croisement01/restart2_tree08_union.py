"""Exact rooted-section unions with independent surface and containment guards."""
import math
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree


def mesh_arrays(obj):
    return np.array([v.co[:] for v in obj.data.vertices],float),[list(p.vertices) for p in obj.data.polygons]


def topology(obj):
    vertices,faces=mesh_arrays(obj);edges={};directed={};adj=[set() for _ in vertices];volume=0.;degenerate=0;center=vertices.mean(0)
    for face in faces:
        for j in range(1,len(face)-1):
            a,b,c=vertices[[face[0],face[j],face[j+1]]];normal=np.cross(b-a,c-a);degenerate+=np.linalg.norm(normal)<1e-9;volume+=np.dot(a-center,normal)/6
        for a,b in zip(face,face[1:]+face[:1]):
            key=tuple(sorted((a,b)));edges[key]=edges.get(key,0)+1;directed[a,b]=directed.get((a,b),0)+1;adj[a].add(b);adj[b].add(a)
    visited=set();components=0
    for i in range(len(vertices)):
        if i in visited:continue
        components+=1;stack=[i];visited.add(i)
        while stack:
            for j in adj[stack.pop()]:
                if j not in visited:visited.add(j);stack.append(j)
    return dict(vertices=len(vertices),faces=len(faces),signed_volume=float(volume),zero_area_triangles=int(degenerate),nonmanifold_edges=sum(count!=2 for count in edges.values()),inconsistent_edge_winding=sum(directed.get((a,b),0)!=directed.get((b,a),0) for a,b in edges),components=components)


def inside(tree,point,epsilon=.001):
    direction=Vector((.371,.529,.763)).normalized();cursor=Vector(point)+direction*epsilon;hits=0
    for _ in range(256):
        hit=tree.ray_cast(cursor,direction,10000)[0]
        if hit is None:return hits%2==1
        hits+=1;cursor=hit+direction*epsilon
    raise RuntimeError('Containment ray did not converge')


def precise_surface_distance(points,vertices,faces,tree):
    # BVH nearest positions are float32 and lose precision at large map
    # coordinates. Keep its candidate face, then measure in float64 geometry.
    indices=[tree.find_nearest(Vector(point))[2] for point in points]
    assert all(i is not None for i in indices)
    triangles=vertices[np.asarray(faces)[indices]];a=triangles[:,0];b=triangles[:,1];c=triangles[:,2];ab=b-a;ac=c-a;normal=np.cross(ab,ac);nn=np.einsum('ij,ij->i',normal,normal)
    projected=points-normal*(np.einsum('ij,ij->i',points-a,normal)/np.maximum(nn,1e-30))[:,None]
    ap=projected-a;aa=np.einsum('ij,ij->i',ab,ab);bb=np.einsum('ij,ij->i',ac,ac);cross=np.einsum('ij,ij->i',ab,ac);pa=np.einsum('ij,ij->i',ap,ab);pc=np.einsum('ij,ij->i',ap,ac);den=aa*bb-cross*cross
    u=(pa*bb-pc*cross)/np.maximum(den,1e-30);v=(pc*aa-pa*cross)/np.maximum(den,1e-30)
    distance=np.where((den>1e-20)&(u>=0)&(v>=0)&(u+v<=1),np.linalg.norm(points-projected,axis=1),np.inf)
    for start,end in [(a,b),(b,c),(c,a)]:
        edge=end-start;t=np.clip(np.einsum('ij,ij->i',points-start,edge)/np.maximum(np.einsum('ij,ij->i',edge,edge),1e-30),0,1);distance=np.minimum(distance,np.linalg.norm(points-start-edge*t[:,None],axis=1))
    return float(distance.max())


def union_sections(bpy,objects,plan,guard):
    results=[];receipts=[];epsilon=.001
    for number,group in enumerate(plan['groups']):
        original=[]
        for index in group:
            vertices,faces=mesh_arrays(objects[index]);original.append(dict(index=index,vertices=vertices,tree=BVHTree.FromPolygons([Vector(v) for v in vertices],faces),lo=vertices.min(0),hi=vertices.max(0),topology=topology(objects[index])))
        if len(group)==1:results.append(objects[group[0]]);continue
        base=max(original,key=lambda x:x['topology']['signed_volume']);obj=objects[base['index']];operand=bpy.data.collections.new(f'Exact rooted fork operands {number}');bpy.context.scene.collection.children.link(operand)
        for index in group:
            if index!=base['index']:operand.objects.link(objects[index])
        guard();bpy.context.view_layer.objects.active=obj;modifier=obj.modifiers.new(f'Exact rooted attachment union {number}','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.operand_type='COLLECTION';modifier.collection=operand;modifier.use_self=True
        print('UNION START',number,'sections',len(group),flush=True);bpy.ops.object.modifier_apply(modifier=modifier.name)
        bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free();obj.data.update()
        precleanup=topology(obj);cleanup_displacement=0.;cleanup_trials=[];cleanup_tolerance=0.
        if precleanup['zero_area_triangles'] or precleanup['nonmanifold_edges']:
            before_cleanup=np.array([v.co[:] for v in obj.data.vertices]);baseline_mesh=obj.data.copy();passed=False
            for tolerance in [1e-6,1e-5,2.5e-5,5e-5,.0001]:
                previous=obj.data;obj.data=baseline_mesh.copy();bpy.data.meshes.remove(previous)
                bm=bmesh.new();bm.from_mesh(obj.data)
                bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=tolerance)
                bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=tolerance)
                bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free();obj.data.update()
                cleaned=BVHTree.FromPolygons([v.co for v in obj.data.vertices],[list(p.vertices) for p in obj.data.polygons]);cleanup_vertices,cleanup_faces=mesh_arrays(obj)
                displacement=precise_surface_distance(before_cleanup,cleanup_vertices,cleanup_faces,cleaned);candidate=topology(obj)
                valid=candidate['components']==1 and not any(candidate[k] for k in ['zero_area_triangles','nonmanifold_edges','inconsistent_edge_winding'])
                cleanup_trials.append(dict(tolerance=tolerance,max_surface_displacement=displacement,topology=candidate))
                if valid and displacement<=.0002:
                    cleanup_displacement=displacement;cleanup_tolerance=tolerance;passed=True;break
            bpy.data.meshes.remove(baseline_mesh)
            assert passed,('No numerical cleanup satisfied unchanged topology/displacement limits',cleanup_trials)
        check=topology(obj)
        assert check['signed_volume']>=max(x['topology']['signed_volume'] for x in original)*.999,('Union lost major volume',check)
        assert check['components']==1 and not any(check[k] for k in ['zero_area_triangles','nonmanifold_edges','inconsistent_edge_winding']),('Union topology failed',check)
        vertices,faces=mesh_arrays(obj);tree=BVHTree.FromPolygons([Vector(v) for v in vertices],faces);lost=[]
        for source in original:
            for i,point in enumerate(source['vertices']):
                nearest=tree.find_nearest(Vector(point))
                if nearest[0] is None or (nearest[3]>epsilon and not inside(tree,point)):
                    lost.append([source['index'],i]);break
        assert not lost,('Union excludes original surface vertices',lost)
        # Every union face centroid must be outside or on the boundary of each
        # input solid. Strictly interior centroids expose retained internal
        # caps/surfaces even when each individual shell is closed and manifold.
        bounds_lo=np.array([x['lo'] for x in original]);bounds_hi=np.array([x['hi'] for x in original]);internal=[];tested=0
        for face_index,face in enumerate(faces):
            point=vertices[face].mean(0);candidates=np.flatnonzero(np.all((point>bounds_lo+epsilon)&(point<bounds_hi-epsilon),axis=1))
            for index in candidates:
                source=original[index];nearest=source['tree'].find_nearest(Vector(point))
                if nearest[0] is None or nearest[3]<=epsilon:continue
                tested+=1
                if inside(source['tree'],point):internal.append([face_index,source['index']]);break
        assert not internal,('Union retains internal face centroids',internal[:20],len(internal))
        receipts.append(dict(group=group,precleanup_topology=precleanup,numerical_cleanup_tolerance=cleanup_tolerance,numerical_cleanup_trials=cleanup_trials,numerical_cleanup_max_surface_displacement=cleanup_displacement,topology=check,all_original_vertices_contained=True,strict_interior_face_centroids=len(internal),interior_queries=tested,tolerance=epsilon,limitation='All face centroids tested against original solids; this is numerical evidence, not a formal proof for every point on a face.'))
        print('UNION PASS',number,check,flush=True)
        for index in group:
            if index==base['index']:continue
            oldmesh=objects[index].data;bpy.data.objects.remove(objects[index],do_unlink=True);bpy.data.meshes.remove(oldmesh)
        bpy.data.collections.remove(operand);results.append(obj)
    results.extend(objects[index] for index in plan['held_sections'])
    return results,receipts
