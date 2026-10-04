"""Native small ground plants with rooted, curved blades and fern fronds."""
import math
from pathlib import Path
import numpy as np
from PIL import Image
from native_foliage import RAY, SIN, COS, material, one_sided, replace_mesh
from native_opacity_bounds import measure


def build(obj, packet):
    directory=Path(packet['directory'])
    rgba=np.asarray(Image.open(directory/'complete-source.png').convert('RGBA'))
    observed=np.asarray(Image.open(directory/'observed-source.png').convert('RGBA'))
    alpha=rgba[:,:,3]>127;yy,xx=np.nonzero(alpha)
    x0,y0,w,h=packet['native_bbox'];ground=float(packet['ground_z'])
    grass=packet['plant_kind']=='grass'
    cx=x0+(xx.min()+xx.max()+1)/2;bottom=y0+yy.max()+1
    width=float(xx.max()-xx.min()+1);height=float(yy.max()-yy.min()+1)
    root=np.array([cx,-(bottom-height*.18+ground*COS)/SIN,ground+.05])
    ray=np.asarray(RAY);rng=np.random.default_rng(80100+packet['native_mask'])
    # The source mask gates inferred front faces, independently of the RGB
    # of the blade. Reverse faces carry only inferred ownership.
    mats=[material(obj.name+' native observed',directory/'observed-source.png',True),
          material(obj.name+' native inferred',directory/'complete-source.png',False)]
    for mat in mats:
        one_sided(mat)
        for node in mat.node_tree.nodes:
            if node.type=='TEX_IMAGE':node.extension='CLIP'
    known_pixels=np.column_stack(np.nonzero(observed[:,:,3]>127))
    palette=np.asarray([observed[y,x,:3] for y,x in known_pixels],dtype=np.uint8)
    palette_img=np.zeros((1,len(palette),4),np.uint8);palette_img[0,:,:3]=palette;palette_img[:,:,3]=255
    Image.fromarray(palette_img).save(directory/'native-palette.png')
    mats.append(material(obj.name+' inferred rooted blades',directory/'native-palette.png',False));one_sided(mats[2])
    blade_count=85 if grass else 11
    skeletons=[]
    for b in range(blade_count):
        angle=math.tau*(b/blade_count)+rng.uniform(-.15,.15)
        outward=np.array([math.cos(angle),math.sin(angle),0.])
        side=np.array([-outward[1],outward[0],0.])
        length=width*rng.uniform(.25,.53);rise=height*rng.uniform(.3,.85)
        base=root+outward*rng.uniform(0,width*.07)
        centers=[base+outward*(length*t**1.35)+np.array([0,0,rise*math.sin(t*math.pi*.72)]) for t in np.linspace(0,1,7)]
        skeletons.append((outward,side,length,centers))
    samples=np.asarray([p for _,_,_,centers in skeletons for p in centers])
    projected=np.column_stack([samples[:,0],-samples[:,1]*SIN-samples[:,2]*COS])
    vertices=[];faces=[];uvs=[];slots=[];known=[]
    def triangle(points,coords,slot,owns=False,reverse=False):
        pts=list(points);uv=list(coords)
        normal=np.cross(pts[1]-pts[0],pts[2]-pts[0]);front=np.dot(normal,ray)>0
        if front==reverse:pts.reverse();uv.reverse()
        k=len(vertices);vertices.extend([list(p) for p in pts]);uvs.extend(uv)
        faces.append((k,k+1,k+2));slots.append(slot);known.append(owns)
    def screen_uv(p):return ((p[0]-x0)/w,1-(-p[1]*SIN-p[2]*COS-y0)/h)
    def source_point(x,y):
        # Place each native pixel near the closest projected rooted blade.
        # Its hidden depth is inferred; the one-pixel fragment retains the
        # exact source coordinate and never bridges separate blade depths.
        nearest=int(np.argmin(np.sum((projected-[x,y])**2,axis=1)))
        z=max(ground+.5,samples[nearest,2]+(projected[nearest,1]-y)*.25/COS)
        return np.array([x,(-y-z*COS)/SIN,z])
    for y,x in zip(yy,xx):
        xy=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)]
        center=source_point(x0+x+.5,y0+y+.5)
        points=[center+np.array([a-x-.5,-(b-y-.5)*SIN,-(b-y-.5)*COS]) for a,b in xy]
        coords=[(a/w,1-b/h) for a,b in xy]
        for face in [(0,1,2),(0,2,3)]:
            pts=[points[i] for i in face];uv=[coords[i] for i in face]
            triangle([p+ray*.02 for p in pts],uv,0,True)
            triangle(pts,uv,1)
            triangle([p-ray*.01 for p in pts],uv,1,False,True)
    # Every inferred blade has a connected root and tapered end. Front faces
    # sample their own native source mask, so they cannot invent observed pixels.
    for outward,side,length,centers in skeletons:
        color=int(rng.integers(len(palette)));uvcolor=((color+.5)/len(palette),.5)
        for j in range(6):
            t=j/6;nt=(j+1)/6
            radius=(.30 if grass else .48)*(1-t)+.04
            nradius=(.30 if grass else .48)*(1-nt)+.015
            pts=[centers[j]-side*radius,centers[j]+side*radius,centers[j+1]+side*nradius,centers[j+1]-side*nradius]
            for face in [(0,1,2),(0,2,3)]:
                tri=[pts[i] for i in face]
                triangle(tri,[screen_uv(p) for p in tri],1)
                triangle(tri,[uvcolor]*3,2,reverse=True)
            if not grass and j>=1:
                # Paired leaflets follow each fern rachis, with real tapered
                # outlines rather than a rectangular texture donor.
                for sign in [-1,1]:
                    start=centers[j];tip=start+side*sign*length*.25*(1-t)+outward*length*.11
                    mid=(start+tip)/2+np.array([0,0,.7]);r=length*.037*(1-t)
                    leaf=[start,mid-outward*r,tip,mid+outward*r]
                    for face in [(0,1,2),(0,2,3)]:
                        tri=[leaf[i] for i in face]
                        triangle(tri,[screen_uv(p) for p in tri],1)
                        triangle(tri,[uvcolor]*3,2,reverse=True)
    result=replace_mesh(obj,vertices,faces,uvs,mats,slots,known)
    obj['projection_component']='crown';obj['projection_preserve']=True;obj['foliage_physical_opacity']=True
    result.update(geometry_version='native-rooted-ground-plants-v4',native_mask=packet['native_mask'],
                  plant_kind='dry grass' if grass else 'fern',references=[],ground_z=ground,
                  minimum_z=min(v.co.z for v in obj.data.vertices),root_world=root.tolist(),
                  blade_count=blade_count,source_projection_preserved=True,opacity_bounds=measure(obj),
                  limitations=['Native silhouette retained by subpixel source fragments; hidden rooted blades are inferred.',
                               'Leaf backs use only this plant native palette, not observed rear artwork.'])
    return result
