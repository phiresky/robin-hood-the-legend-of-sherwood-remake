"""Apply a continuous piecewise-affine ray-depth map with exact face splitting."""
import numpy as np
import bpy
from mathutils import Vector
from tree_geometry import SIN,COS,RAY


def bend(obj):
    old=obj.data;old.calc_loop_triangles()
    uvs=list(old.uv_layers);colors=list(old.color_attributes)
    assert all(c.domain=='CORNER' for c in colors)
    def attrs(index,loop):
        return np.array([*tuple(obj.matrix_world@old.vertices[index].co),*[v for uv in uvs for v in uv.data[loop].uv],*[v for c in colors for v in c.data[loop].color]],dtype=float)
    # Shift is min of these affine bounds inside their common positive region.
    forms=np.array([[0,0,1.04,-.52],[8,0,0,-320],[-8,0,0,1000],[0,-8*SIN,-8*COS,-1760],[0,8*SIN,8*COS,2560],[0,0,0,115]],dtype=float)
    def value(p,f):return p[:3]@f[:3]+f[3]
    def clip(poly,f):
        yes=[];no=[]
        if not poly:return yes,no
        for a,b in zip(poly,poly[1:]+poly[:1]):
            va,vb=value(a,f),value(b,f);inside=va>=-1e-9
            (yes if inside else no).append(a)
            if (va>1e-9 and vb < -1e-9) or (va < -1e-9 and vb>1e-9):
                p=a+(b-a)*(va/(va-vb));yes.append(p);no.append(p)
        return yes,no
    vertices=[];faces=[];records=[];mats=[];maxshift=0
    def emit(poly,form,material):
        nonlocal maxshift
        if len(poly)<3:return
        projected=np.array([[p[0],-SIN*p[1]-COS*p[2]] for p in poly])
        # Some source edge-on internal cards still have valid world area.
        if sum(np.linalg.norm(np.cross(poly[i][:3]-poly[0][:3],poly[i+1][:3]-poly[0][:3])) for i in range(1,len(poly)-1))<1e-10:return
        start=len(vertices)
        for p in poly:
            d=max(0,value(p,form));maxshift=max(maxshift,d)
            vertices.append(tuple(obj.matrix_world.inverted()@Vector(p[:3]-np.array(RAY)*d)));records.append(p[3:])
        faces.append(tuple(range(start,len(vertices))));mats.append(material)
    zero=np.zeros(4)
    for tri in old.loop_triangles:
        inner=[attrs(i,l) for i,l in zip(tri.vertices,tri.loops)]
        for f in forms:
            inner,outer=clip(inner,f);emit(outer,zero,tri.material_index)
            if len(inner)<3:break
        if len(inner)<3:continue
        for i,f in enumerate(forms):
            cell=inner
            for j,g in enumerate(forms):
                if i==j:continue
                cell,_=clip(cell,g-f)
                if len(cell)<3:break
            emit(cell,f,tri.material_index)
    mesh=bpy.data.meshes.new(old.name+' / coherent depth cells');mesh.from_pydata(vertices,[],faces);mesh.update()
    for m in old.materials:mesh.materials.append(m)
    for p,m in zip(mesh.polygons,mats):p.material_index=m
    offset=0
    for layer in uvs:
        uv=mesh.uv_layers.new(name=layer.name)
        for loop in mesh.loops:uv.data[loop.index].uv=records[loop.vertex_index][offset:offset+2]
        offset+=2
    for layer in colors:
        color=mesh.color_attributes.new(name=layer.name,type=layer.data_type,domain='CORNER')
        for loop in mesh.loops:color.data[loop.index].color=records[loop.vertex_index][offset:offset+4]
        offset+=4
    for key in old.keys():mesh[key]=old[key]
    obj.data=mesh
    return dict(original_vertices=len(old.vertices),original_faces=len(old.polygons),new_vertices=len(mesh.vertices),new_faces=len(mesh.polygons),maximum_shift=maxshift,minimum_source_ray_jacobian=1-SIN*1.04,method='Grounded source-ray map: min of affine height, spatial taper, and maximum-retreat bounds. Every triangle is split at region boundaries before deformation. Same monotone point map applies to observed faces, paired backs and interior leaves.',source_bounds=[40,220,125,320],height_coefficient=1.04,maximum_retreat_bound=115)
