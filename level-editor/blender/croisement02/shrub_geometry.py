"""Small native-leaf clumps with rounded world-space depth and explicit backs."""
import math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt, binary_erosion
from tree_geometry import SIN, COS, RAY, material, one_sided, replace_mesh
from opacity_bounds import measure


def build(obj, packet):
    directory=Path(packet['directory'])
    rgba=np.asarray(Image.open(directory/'complete-source.png').convert('RGBA'))
    visible=np.asarray(Image.open(directory/'observed-source.png').convert('RGBA'))
    alpha=rgba[:,:,3]>127; known_alpha=visible[:,:,3]>127
    interior_alpha=binary_erosion(alpha,iterations=1)
    if known_alpha.sum()<200:raise ValueError('Insufficient uncontaminated native leaf evidence')
    _,nearest=distance_transform_edt(~known_alpha,return_indices=True)
    inferred=visible[nearest[0],nearest[1]].copy();inferred[:,:,3]=rgba[:,:,3]
    if packet.get('inferred_front_image'):
        provisional=np.asarray(Image.open(directory/packet['inferred_front_image']).convert('RGBA'))
        if provisional.shape!=inferred.shape:raise ValueError('Inferred front dimensions differ')
        inferred[~known_alpha]=provisional[~known_alpha]
        inferred[:,:,3]=rgba[:,:,3]
    Image.fromarray(inferred).save(directory/'inferred-source.png')
    unknown=inferred.copy();unknown[:,:,3]=np.where(known_alpha,0,rgba[:,:,3])
    Image.fromarray(unknown).save(directory/'unknown-front.png')
    mats=[material(obj.name+' observed leaves',directory/'observed-source.png',True),
          material(obj.name+' inferred covered front',directory/'unknown-front.png',False),
          material(obj.name+' inferred leaf backs',directory/'inferred-source.png',False)]
    for mat in mats:one_sided(mat)
    x0,y0,width,height=packet['native_bbox']
    yy,xx=np.nonzero(alpha);fw=float(xx.max()-xx.min()+1);fh=float(yy.max()-yy.min()+1)
    cx=x0+(xx.min()+xx.max()+1)/2;cy=y0+(yy.min()+yy.max()+1)/2
    radii=np.array([fw*.51,fw*.57,max(fh*.27,math.sqrt(max(4,(fh*.52)**2-(fw*.57*SIN)**2))/COS)])
    center=np.array([cx,(-cy-(radii[2]+1)*COS)/SIN,radii[2]+1]);ray=np.asarray(RAY)
    vertices=[];faces=[];uvs=[];slots=[];known=[]
    def point(x,y,d):return center+np.array([x-cx,-(y-cy)*SIN,-(y-cy)*COS])+ray*d
    def quad(points,coords,slot,observed=False,reverse=False):
        start=len(vertices);vertices.extend([list(p) for p in points]);uvs.extend(coords)
        faces.extend([(start+2,start+1,start),(start+3,start+2,start)] if reverse else [(start,start+1,start+2),(start,start+2,start+3)])
        slots.extend([slot,slot]);known.extend([observed,observed])
    # The observed source is divided into small fixed patches along the front
    # of a round volume; one shell avoids repeated broad parallel image layers.
    a=np.sum((ray/radii)**2)
    for top in range(0,height,6):
        for left in range(0,width,6):
            right,bottom=min(width,left+7),min(height,top+7)
            if not alpha[top:bottom,left:right].any():continue
            xa,xb,ya,yb=x0+left,x0+right,y0+top,y0+bottom
            relative=point((xa+xb)/2,(ya+yb)/2,0)-center
            b=2*np.sum(relative*ray/radii**2);c=np.sum((relative/radii)**2)-1
            depth=(-b+math.sqrt(max(0,b*b-4*a*c)))/(2*a)
            pts=[point(xa,ya,depth),point(xb,ya,depth),point(xb,yb,depth),point(xa,yb,depth)]
            if packet.get('curved_front'):
                pts=[]
                for sx,sy in [(xa,ya),(xb,ya),(xb,yb),(xa,yb)]:
                    relative=point(sx,sy,0)-center
                    b=2*np.sum(relative*ray/radii**2);c=np.sum((relative/radii)**2)-1
                    pts.append(point(sx,sy,(-b+math.sqrt(max(0,b*b-4*a*c)))/(2*a)))
            uv=[(left/width,1-top/height),(right/width,1-top/height),(right/width,1-bottom/height),(left/width,1-bottom/height)]
            quad(pts,uv,0,True,True)
            quad([p-ray*.001 for p in pts],uv,1,False,True)
            quad([p-ray*.02 for p in pts],uv,2)
    patches=[(x,y) for y in range(0,height-12,3) for x in range(0,width-12,3) if known_alpha[y:y+12,x:x+12].mean()>.25]
    if not patches:raise ValueError('No native leaf patches')
    rng=np.random.default_rng(55100+packet['native_mask']);tiles=[]
    sample_v,sample_u=np.mgrid[0:24,0:24]/24+.5/24
    cluster_count=int(packet.get('cluster_count',450));atlas_rows=max(64,math.ceil(cluster_count*6/48))
    for _ in range(cluster_count):
        unit=rng.normal(size=3);unit/=np.linalg.norm(unit);unit*=rng.uniform(.03,1)**(1/3)
        pos=center+unit*radii*.92
        axis=rng.normal(size=3);axis/=np.linalg.norm(axis)
        other=np.cross(axis,[0,0,1] if abs(axis[2])<.9 else [1,0,0]);other/=np.linalg.norm(other)
        third=np.cross(axis,other);size=float(rng.uniform(*packet.get('leaf_size_range',[3.5,6.])))
        px,py=patches[int(rng.integers(len(patches)))]
        leaf=np.asarray(Image.fromarray(inferred[py:py+12,px:px+12]).resize((24,24),Image.Resampling.NEAREST)).copy()
        for u,v in [(axis,other),(axis,third),(other,third)]:
            pts=[pos+(u*su+v*sv)*size for su,sv in [(-1,-1),(1,-1),(1,1),(-1,1)]]
            p0,p1,p2,p3=pts
            sample=(1-sample_u)[...,None]*(1-sample_v)[...,None]*p0+sample_u[...,None]*(1-sample_v)[...,None]*p1+sample_u[...,None]*sample_v[...,None]*p2+(1-sample_u)[...,None]*sample_v[...,None]*p3
            ix=np.floor(sample[:,:,0]-x0).astype(int);iy=np.floor(-sample[:,:,1]*SIN-sample[:,:,2]*COS-y0).astype(int)
            valid=(ix>=0)&(ix<width)&(iy>=0)&(iy<height)
            gate=np.zeros((24,24),bool);gate[valid]=interior_alpha[iy[valid],ix[valid]]
            tile=leaf.copy();tile[:,:,3]=np.where(gate,tile[:,:,3],0)
            def tile_quad(tile_image, points, slot):
                index=len(tiles);tiles.append(tile_image);column,row=index%48,index//48
                uv=[(column/48,1-row/atlas_rows),((column+1)/48,1-row/atlas_rows),((column+1)/48,1-(row+1)/atlas_rows),(column/48,1-(row+1)/atlas_rows)]
                quad(points,uv,slot)
            if np.any(tile[:,:,3]>127):tile_quad(tile,pts,3)
            # Unseen leaves on the rear half fill the long source-ray holes.
            # Their one-sided faces point away from the source camera: they
            # cannot manufacture observed front pixels outside the native mask.
            back_points=list(pts)
            if np.dot(np.cross(back_points[1]-back_points[0],back_points[2]-back_points[0]),ray)>0:
                back_points.reverse()
            tile_quad(leaf.copy(),back_points,4)
    atlas=np.zeros((atlas_rows*24,48*24,4),dtype=np.uint8)
    for i,tile in enumerate(tiles):atlas[(i//48)*24:(i//48+1)*24,(i%48)*24:(i%48+1)*24]=tile
    Image.fromarray(atlas).save(directory/'inferred-volume.png')
    mats.append(material(obj.name+' inferred interior leaves',directory/'inferred-volume.png',False))
    mats.append(material(obj.name+' inferred rear leaf volume',directory/'inferred-volume.png',False))
    one_sided(mats[4])
    result=replace_mesh(obj,vertices,faces,uvs,mats,slots,known)
    obj['projection_component']='crown';obj['projection_preserve']=True;obj['foliage_physical_opacity']=True
    bounds=measure(obj)
    for _ in range(3):
        if bounds['depth_width_ratio']>=1.02:break
        scale=1.04/bounds['depth_width_ratio']
        for vertex in obj.data.vertices:
            p=np.asarray(vertex.co);d=float(np.dot(p-center,ray));vertex.co=p+ray*d*(scale-1)
        obj.data.update();bounds=measure(obj)
    minimum=min(v.co.z for v in obj.data.vertices)
    if minimum<.5:
        for v in obj.data.vertices:v.co+=RAY*((.5-minimum)/SIN)
    result.update(geometry_version='native-shrub-leaf-volume-v2',native_mask=packet['native_mask'],
        source_projection_preserved=True,observed_leaf_pixels=int(known_alpha.sum()),inferred_covered_pixels=int((alpha&~known_alpha).sum()),
        leaf_clusters=len(tiles),opacity_bounds=measure(obj),minimum_z=min(v.co.z for v in obj.data.vertices),
        method='Small observed front cutouts on a round world volume; source-clipped interior leaves and one-sided inferred rear volume',
        references=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'])
    return result
