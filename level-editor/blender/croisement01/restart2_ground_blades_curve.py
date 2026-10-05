"""Continuous native leaf depth fields with curved narrow inferred supports."""
import math
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import label
from mathutils import Vector
from tree_geometry import material,one_sided,replace_mesh,RAY,SIN,COS


def build(obj,source,guide,ground):
    source=Path(source);rgba=np.asarray(Image.open(source/'native.png').convert('RGBA'));alpha=rgba[:,:,3]>127
    x0,y0,w,h=guide['native_bbox'];root=np.asarray(guide['root_source_local'],float)
    ground_paths=set()
    paths=[];curves=[]
    for index,raw in enumerate(guide['paths']):
        p=np.asarray([root,*list(reversed(raw))],float)+.5
        p[0]=root
        for _ in range(2):p[1:-1]=(p[:-2]+2*p[1:-1]+p[2:])/4
        keep=np.r_[True,np.linalg.norm(np.diff(p,axis=0),axis=1)>1e-5];p=p[keep]
        arc=np.r_[0,np.cumsum(np.linalg.norm(np.diff(p,axis=0),axis=1))];arc/=arc[-1]
        length=np.linalg.norm(p[-1]-root);height=max(5,length*.82)*(1+.13*math.sin(index*2.39996))
        paths.append(p);curves.append((arc,height))
    def point(px,py,leaf):
        p=paths[leaf];arc,height=curves[leaf];q=np.asarray([px-x0,py-y0]);a=p[:-1];d=p[1:]-a
        f=np.clip(np.sum((q-a)*d,axis=1)/np.maximum(np.sum(d*d,axis=1),1e-8),0,1)
        axis=p[-1]-root;t=float(np.clip(np.dot(q-root,axis)/max(np.dot(axis,axis),1e-8),0,1))
        low=axis[1]>-8
        z=ground+.06+height*(.28*math.sin(math.pi*t) if low else math.sin(math.pi*.55*t))
        return Vector((px,-(py+z*COS)/SIN,z))
    mats=[material(obj.name+' protected observed blades',source/'native.png',True),material(obj.name+' inferred blade fronts',source/'native.png',True)]
    known_pixels=rgba[alpha];palette=known_pixels[(known_pixels[:,1]>60)&(known_pixels[:,0]>55)&(known_pixels[:,1]>known_pixels[:,2]*1.3)]
    if len(palette)<8:raise ValueError('Insufficient own-plant palette')
    Image.fromarray(palette.reshape(1,-1,4)).save(source/'blade-support-palette.png');mats.append(material(obj.name+' inferred continuous blade backs',source/'blade-support-palette.png',False))
    for mat in mats:
        one_sided(mat)
        for node in mat.node_tree.nodes:
            if node.type=='TEX_IMAGE':node.extension='CLIP'
    vertices=[];faces=[];uvs=[];slots=[];owns=[];keys={}
    def tri(points,coords,slot,known=False,back=False,shared=None):
        points=list(points);coords=list(coords)
        if ((points[1]-points[0]).cross(points[2]-points[0]).dot(Vector(RAY))>0)==back:points.reverse();coords.reverse();shared=list(reversed(shared)) if shared else None
        face=[]
        for i,p in enumerate(points):
            key=shared[i] if shared else None
            if key is not None and key in keys:idx=keys[key]
            else:
                idx=len(vertices);vertices.append(tuple(p));uvs.append(coords[i])
                if key is not None:keys[key]=idx
            face.append(idx)
        faces.append(tuple(face));slots.append(slot);owns.append(known)
    for pixel in guide['observed_pixels']:
        x,y,leaf=pixel['x'],pixel['y'],pixel['leaf'];corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)]
        points=[point(x0+a,y0+b,leaf)+Vector(RAY)*.018 for a,b in corners];coords=[(a/w,1-b/h) for a,b in corners]
        for ids in ((0,1,2),(0,2,3)):tri([points[i] for i in ids],[coords[i] for i in ids],0,True,shared=[(corners[i][0],corners[i][1],leaf) for i in ids])
    rng=np.random.default_rng(guide['native_mask']+15000)
    for leaf,p in enumerate(paths):
        if leaf in ground_paths:continue
        arc,height=curves[leaf];color=int(rng.integers(len(palette)));pal=((color+.5)/len(palette),.5);rings=[]
        for j,q in enumerate(p):
            tangent=p[min(j+1,len(p)-1)]-p[max(j-1,0)];tangent/=max(np.linalg.norm(tangent),1e-8);side=np.asarray([-tangent[1],tangent[0]])
            width=.16+1.05*math.sin(math.pi*arc[j])**.6
            rings.append([q-side*width,q+side*width])
        for j in range(len(p)-1):
            xy=[rings[j][0],rings[j][1],rings[j+1][1],rings[j+1][0]];points=[point(x0+q[0],y0+q[1],leaf) for q in xy];coords=[(q[0]/w,1-q[1]/h) for q in xy]
            for ids in ((0,1,2),(0,2,3)):
                tri([points[i] for i in ids],[coords[i] for i in ids],1,True)
                tri([points[i]-Vector(RAY)*.012 for i in ids],[pal]*3,2,False,True)
    # Additional radial leaves provide hidden volume; each has a real rooted
    # curved strip. Front alpha remains constrained by the untouched native map.
    root_world=point(x0+root[0],y0+root[1],0)
    for blade in range(30):
        angle=blade*math.tau/30;outward=Vector((math.cos(angle),math.sin(angle),0));side=Vector((-outward.y,outward.x,0));length=w*rng.uniform(.24,.42);rise=h*rng.uniform(.3,.65)
        centers=[root_world+outward*(length*t**1.4)+Vector((0,0,rise*math.sin(t*math.pi*.78))) for t in np.linspace(0,1,9)];color=int(rng.integers(len(palette)));pal=((color+.5)/len(palette),.5)
        for j in range(8):
            width=.65*(1-j/8)+.06;next_width=.65*(1-(j+1)/8)+.02;points=[centers[j]-side*width,centers[j]+side*width,centers[j+1]+side*next_width,centers[j+1]-side*next_width];coords=[((p.x-x0)/w,1-(-p.y*SIN-p.z*COS-y0)/h) for p in points]
            for ids in ((0,1,2),(0,2,3)):
                tri([points[i] for i in ids],[coords[i] for i in ids],1,True)
                tri([points[i]-Vector(RAY)*.012 for i in ids],[pal]*3,2,False,True)
    result=replace_mesh(obj,vertices,faces,uvs,mats,slots,owns);components,total=label(alpha,np.ones((3,3)));sizes=np.bincount(components.ravel())[1:]
    result.update(geometry_version='continuous-native-leaf-supports-v4-monotone-path' ,ground_path_hypothesis=sorted(ground_paths),ground_domain_status='No native ground ownership reclassification; low leaves use rooted arch curves' ,native_mask=guide['native_mask'],rooted_source_paths=len(paths),extra_inferred_blades=30,native_connected_components=int(total),native_component_sizes=sorted(sizes.tolist(),reverse=True),source_rgba_changed=False,limitations=['Leaf associations and hidden curvature are inferred from native leaf paths.','Native isolated source tips remain isolated in alpha, with narrow inferred reverse supports.','Source painted low basal regions still require semantic review.'])
    return result
