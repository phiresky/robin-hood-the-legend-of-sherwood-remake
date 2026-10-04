"""Closed rounded volumes for independently observed native wood fragments."""
import math
import bpy,bmesh
import numpy as np
from scipy.ndimage import label,find_objects,distance_transform_edt
from tree_geometry import SIN,COS,RAY
from mathutils import Vector

def build_fragments(alpha,box,material,gray,scene):
    labels,count=label(alpha);objects=[];report=[];height,width=alpha.shape
    for component,extent in enumerate(find_objects(labels),1):
        if extent is None:continue
        mask=labels[extent]==component;ys,xs=np.where(mask);area=len(xs)
        if area>2:
            covariance=np.cov(np.column_stack([xs,ys]).T);values,vectors=np.linalg.eigh(covariance);short_width=max(1,math.sqrt(max(values[0],0))*4);minor=vectors[:,0]
        else:short_width=1;minor=np.array([0.,1.])
        # Wood-only source components determine width; depth is explicitly inferred.
        half_depth=max(.65,short_width/2);ground_height=half_depth*float(RAY.z)+1.2
        centroid=np.array([xs.mean()+.5,ys.mean()+.5]);minor_world=Vector((float(minor[0]),-float(minor[1])/SIN,0))
        distance=distance_transform_edt(mask);max_distance=float(distance.max());vertices=[];faces=[];corner_indices={};cell=[0,0]
        def corner(x,y,front):
            neighbors=[(xx,yy) for yy in (y-1,y) for xx in (x-1,x) if 0<=yy<mask.shape[0] and 0<=xx<mask.shape[1] and mask[yy,xx]]
            pinch=len(neighbors)==2 and neighbors[0][0]!=neighbors[1][0] and neighbors[0][1]!=neighbors[1][1]
            key=(x,y,front,tuple(cell) if pinch else None)
            if key in corner_indices:return corner_indices[key]
            nearby=distance[max(0,y-1):min(mask.shape[0],y+1),max(0,x-1):min(mask.shape[1],x+1)]
            d=float(nearby.mean()) if nearby.size else 0
            depth=max(.35,half_depth*math.sqrt(min(1,d/max_distance)))
            sx=box[0]+extent[1].start+x;sy=box[1]+extent[0].start+y
            base=Vector((sx,-(sy+ground_height*COS)/SIN,ground_height))
            # Remove camera-ray shear across the minor axis while preserving the
            # horizontal long axis and every observed source coordinate.
            transverse=float(np.dot(np.array([x,y])-centroid,minor))
            base-=RAY*(transverse*minor_world.dot(RAY))
            p=base+RAY*depth*(1 if front else -1)
            index=len(vertices);vertices.append(tuple(p));corner_indices[key]=index;return index
        for y,x in zip(ys,xs):
            cell[:]=[int(x),int(y)]
            coordinates=[(x,y),(x,y+1),(x+1,y+1),(x+1,y)]
            faces.append(tuple(corner(a,b,True)for a,b in coordinates));faces.append(tuple(corner(a,b,False)for a,b in reversed(coordinates)))
            for (ax,ay),(bx,by),(dx,dy) in [(coordinates[0],coordinates[1],(-1,0)),(coordinates[1],coordinates[2],(0,1)),(coordinates[2],coordinates[3],(1,0)),(coordinates[3],coordinates[0],(0,-1))]:
                nx,ny=x+dx,y+dy
                if 0<=ny<mask.shape[0] and 0<=nx<mask.shape[1] and mask[ny,nx]:continue
                faces.append((corner(ax,ay,True),corner(ax,ay,False),corner(bx,by,False),corner(bx,by,True)))
        # Rest each inferred volume on ground without moving its source projection.
        lift=(1.2-min(p[2] for p in vertices))/float(RAY.z)
        vertices=[tuple(Vector(p)+RAY*lift) for p in vertices]
        mesh=bpy.data.meshes.new(f'Applied native wood fragment {component:03d}');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bad=sum(not e.is_manifold for e in bm.edges);zero=sum(f.calc_area()<1e-10 for f in bm.faces);badverts=sum(not v.is_manifold for v in bm.verts);assert not bad and not zero and not badverts,(component,bad,zero,badverts);bm.to_mesh(mesh);bm.free();mesh.materials.append(material);mesh.materials.append(gray)
        uv=mesh.uv_layers.new(name='Native target projection')
        for face in mesh.polygons:
            face.material_index=0 if face.normal.dot(RAY)>.05 else 1
            for loop in face.loop_indices:
                p=mesh.vertices[mesh.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-box[0])/width,1-(-p.y*SIN-p.z*COS-box[1])/height)
        mesh.update()
        obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj);obj['state_endpoint']='applied';obj['native_source_tick']=89;obj['geometry_status']='unapproved inferred round fragment depth';objects.append(obj);report.append(dict(component=component,native_pixels=area,inferred_half_depth=half_depth,closed_manifold=True,zero_area_faces=0))
    return objects,report
