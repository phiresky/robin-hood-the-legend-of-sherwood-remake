"""Subdivide only bank-constrained paired leaves while preserving source attributes."""
import copy
from collections import defaultdict
import bpy,numpy as np
from mathutils import Vector
from tree_geometry import SIN,COS
from sign_rigid_depth import groups_for


def refine(obj,detail,conflicts):
    old=obj.data;old.calc_loop_triangles();world=np.array([tuple(obj.matrix_world@v.co) for v in old.vertices]);projection=np.column_stack((world[:,0],-SIN*world[:,1]-COS*world[:,2]));groups,roots=groups_for(old,world,projection)
    selected=set(conflicts);assert selected<=groups.keys()
    layers=list(old.uv_layers);colors=list(old.color_attributes);assert all(c.domain=='CORNER' for c in colors)
    vertices=[];faces=[];values=[];slots=[];identities=[];source_polygons=[];mapping=defaultdict(list);keys={};next_identity=max(groups)+1
    def split(triangle,depth):
        if depth==0:return [triangle]
        a,b,c=triangle;ab=(a+b)/2;bc=(b+c)/2;ca=(c+a)/2
        return [q for t in [[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]] for q in split(t,depth-1)]
    for tri in old.loop_triangles:
        g=roots[tri.vertices[0]];attrs=np.array([[*world[v],*[t for uv in layers for t in uv.data[l].uv],*[t for color in colors for t in color.data[l].color]] for v,l in zip(tri.vertices,tri.loops)])
        for weights in split(np.eye(3),2 if g in selected else 0):
            rows=np.array(weights)@attrs;identity=g
            if g in selected:
                projected=[(round(float(p[0]),2),round(float(-SIN*p[1]-COS*p[2]),2)) for p in rows]
                key=(g,tuple(sorted(projected)))
                if key not in keys:keys[key]=next_identity;next_identity+=1
                identity=keys[key]
            start=len(vertices);vertices.extend([tuple(obj.matrix_world.inverted()@Vector(r[:3])) for r in rows]);values.extend(rows[:,3:]);faces.append((start,start+1,start+2));slots.append(tri.material_index);identities.append(identity);source_polygons.append(tri.polygon_index);mapping[tri.polygon_index].append(len(faces)-1)
    mesh=bpy.data.meshes.new(old.name+' / bounded paired subfragments');mesh.from_pydata(vertices,[],faces);mesh.update()
    for mat in old.materials:mesh.materials.append(mat)
    for polygon,slot in zip(mesh.polygons,slots):polygon.material_index=slot
    offset=0
    for layer in layers:
        uv=mesh.uv_layers.new(name=layer.name)
        for loop in mesh.loops:uv.data[loop.index].uv=values[loop.vertex_index][offset:offset+2]
        offset+=2
    for layer in colors:
        color=mesh.color_attributes.new(name=layer.name,type=layer.data_type,domain='CORNER')
        for loop in mesh.loops:color.data[loop.index].color=values[loop.vertex_index][offset:offset+4]
        offset+=4
    pair=mesh.attributes.new('Sign leaf pair','INT','FACE')
    for i,identity in enumerate(identities):pair.data[i].value=identity
    source_index=mesh.attributes.new('Sign source polygon','INT','FACE')
    for i,identity in enumerate(source_polygons):source_index.data[i].value=identity
    for k in old.keys():mesh[k]=old[k]
    obj.data=mesh
    remapped=copy.deepcopy(detail);newworld=np.array([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);projected=np.column_stack((newworld[:,0],-SIN*newworld[:,1]-COS*newworld[:,2]))
    for row in remapped['pixels']:
        pixel=np.array(row['source_pixel'])+.5
        for hit in row.get('blockers',[]):
            matches=[]
            for index in mapping[hit['polygon']]:
                p=mesh.polygons[index];a,b,c=projected[list(p.vertices)];basis=np.column_stack((b-a,c-a))
                if abs(np.linalg.det(basis))<1e-10:continue
                bc=np.linalg.solve(basis,pixel-a)
                if min(bc)>=-.001 and bc.sum()<=1.001:matches.append(index)
            if not matches:raise ValueError('Source ray lost during paired subdivision')
            hit['polygon']=matches[0]
    return remapped,dict(subdivided_original_pairs=sorted(selected),subdivision_levels=2,original_faces=len(old.polygons),new_faces=len(mesh.polygons),method='Only five constrained pairs subdivided into16 rigid subtriangles per original triangle. Materials remain shared, UV and ownership interpolate original face attributes; explicit face pairing preserves matching fronts and backs.')
