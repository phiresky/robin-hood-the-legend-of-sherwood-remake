"""Append small inferred shrub supports while preserving every existing leaf loop."""
import hashlib
import math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from tree_geometry import SIN, COS, RAY, material, one_sided


def leaf_signature(mesh, vertices, faces, loops):
    digest=hashlib.sha256()
    for values in ([tuple(v.co) for v in mesh.vertices[:vertices]],
                   [tuple(p.vertices) for p in mesh.polygons[:faces]],
                   [p.material_index for p in mesh.polygons[:faces]],
                   [tuple(v.uv) for v in mesh.uv_layers['Foliage UV'].data[:loops]],
                   [tuple(v.color) for v in mesh.color_attributes['Source ownership'].data[:loops]]):
        digest.update(repr(values).encode())
    return digest.hexdigest()


def append_support(obj,packet,ground):
    if packet['native_mask']!=71:raise ValueError('Support recipe reviewed only for native71')
    old=obj.data;nv,np_,nl=len(old.vertices),len(old.polygons),len(old.loops)
    before=leaf_signature(old,nv,np_,nl)
    vertices=[tuple(v.co) for v in old.vertices];faces=[tuple(p.vertices) for p in old.polygons]
    slots=[p.material_index for p in old.polygons]
    uv=[tuple(v.uv) for v in old.uv_layers['Foliage UV'].data]
    colors=[tuple(v.color) for v in old.color_attributes['Source ownership'].data]
    mats=list(old.materials);directory=Path(packet['directory'])
    source=np.asarray(Image.open(directory/'observed-source.png').convert('RGBA'))
    palette=source[source[:,:,3]>127][:,:3].astype(float)
    brown=palette[(palette[:,0]>palette[:,1]*.93)&(palette[:,1]>palette[:,2]*1.1)]
    if not len(brown):raise ValueError('No own native warm twig palette')
    rgb=np.median(brown,axis=0).astype('uint8')
    image=np.zeros((32,32,4),dtype='uint8');image[:,:,:3]=rgb;image[:,:,3]=255
    Image.fromarray(image).save(directory/'inferred-support-back.png')
    front_slot=len(mats)
    mats.extend([material(obj.name+' source-clipped inferred support',directory/'observed-source.png',False),
                 material(obj.name+' native palette inferred support backs',directory/'inferred-support-back.png',False)])
    for mat in mats[-2:]:one_sided(mat)
    # The low inferred leaf clusters determine modest branch endpoints. The
    # original leaf mesh, its source coordinates and all materials stay fixed.
    centers=np.asarray([tuple(p.center) for p in old.polygons if p.material_index in (3,4)])
    low=centers[centers[:,2]<=np.quantile(centers[:,2],.28)]
    targets=[]
    for q in (.25,.5,.75):
        x=np.quantile(low[:,0],q)
        candidates=low[np.abs(low[:,0]-x)<8]
        target=np.median(candidates,axis=0)
        target[2]=max(ground+9,min(ground+15,target[2]))
        targets.append(target)
    x0,y0,w,h=packet['native_bbox'];ray=np.asarray(RAY);branches=[]
    def branch(start,end,r0,r1):
        direction=end-start;direction/=np.linalg.norm(direction)
        u=np.cross(direction,[1,0,0]);u/=np.linalg.norm(u);v=np.cross(direction,u)
        for i in range(7):
            a,b=math.tau*i/7,math.tau*(i+1)/7
            points=[start+r0*(u*math.cos(a)+v*math.sin(a)),start+r0*(u*math.cos(b)+v*math.sin(b)),
                    end+r1*(u*math.cos(b)+v*math.sin(b)),end+r1*(u*math.cos(a)+v*math.sin(a))]
            for p in points:p[2]=max(ground+.5,p[2])
            is_front=np.dot(np.cross(points[1]-points[0],points[2]-points[0]),ray)>0
            coords=[((p[0]-x0)/w,1-(-p[1]*SIN-p[2]*COS-y0)/h) for p in points] if is_front else [(0,0),(1,0),(1,1),(0,1)]
            offset=len(vertices);vertices.extend(tuple(p) for p in points);faces.append(tuple(range(offset,offset+4)))
            slots.append(front_slot+(not is_front));uv.extend(coords);colors.extend([(0.,1.,1.,1.)]*4)
        branches.append(dict(start=start.tolist(),end=end.tolist(),radii=[r0,r1]))
    for k,target in enumerate(targets):
        root=target.copy();root[0]+=(k-1)*1.8;root[2]=ground+.5
        joint=root+(target-root)*.65
        branch(root,target,.55,.12)
        for sign in (-1,1):
            tip=target+np.array([sign*3,sign*1.8,1.5])
            branch(joint,tip,.22,.045)
    mesh=bpy.data.meshes.new(obj.name+' inferred supports')
    mesh.from_pydata(vertices,[],faces);mesh.update()
    for mat in mats:mesh.materials.append(mat)
    layer=mesh.uv_layers.new(name='Foliage UV');ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
    mesh.color_attributes.active_color=ownership
    for p,slot in zip(mesh.polygons,slots):p.material_index=slot
    for i,(coord,color) in enumerate(zip(uv,colors)):
        layer.data[i].uv=coord;ownership.data[i].color=color
    after=leaf_signature(mesh,nv,np_,nl)
    if before!=after:raise ValueError('Existing leaf geometry, UV, material slots or ownership changed')
    obj.data=mesh
    return dict(inferred=True,old_leaf_signature=before,new_leaf_signature=after,leaf_geometry_unchanged=True,
                branches=branches,source_front='Exact own observed RGB/alpha; back faces own native warm palette',
                limitation='Minimal inferred branched support, not observed roots; leaf positions unchanged')
