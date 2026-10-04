"""Own-native conifer silhouette with tapered world-space needle volume."""
import math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt, binary_erosion
from scipy.spatial import Delaunay
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
    # Root and crown height follow this plant's own observed source axis.
    root_source_y=y0+float(yy.max())+.5
    crown_height=fh/COS
    root_y=-root_source_y/SIN
    root=np.array([cx,root_y,.5])
    center=root+np.array([0,0,crown_height*.5]);ray=np.asarray(RAY)
    base_radius=fw*.53
    radii=np.array([base_radius,base_radius,crown_height*.5])
    vertices=[];faces=[];uvs=[];slots=[];known=[]
    center_source_y=-center[1]*SIN-center[2]*COS
    def point(x,y,d):return center+np.array([x-cx,-(y-center_source_y)*SIN,-(y-center_source_y)*COS])+ray*d
    def quad(points,coords,slot,observed=False,reverse=False):
        start=len(vertices);vertices.extend([list(p) for p in points]);uvs.extend(coords)
        faces.extend([(start+2,start+1,start),(start+3,start+2,start)] if reverse else [(start,start+1,start+2),(start,start+2,start+3)])
        slots.extend([slot,slot]);known.extend([observed,observed])
    # The observed source is divided into small fixed patches along the front
    # of an upright taper; one shell avoids repeated broad image layers.
    def front_depth(sx,sy):
        # Intersect the source ray with a true upright taper, keeping every
        # source coordinate unchanged. Fine external sprigs use the closest
        # cone surface when they fall outside its smooth envelope.
        relative=point(sx,sy,0)-root
        k=base_radius/crown_height
        aa=ray[0]**2+ray[1]**2-k*k*ray[2]**2
        bb=2*(relative[0]*ray[0]+relative[1]*ray[1]+k*k*(crown_height-relative[2])*ray[2])
        cc=relative[0]**2+relative[1]**2-k*k*(crown_height-relative[2])**2
        disc=max(0,bb*bb-4*aa*cc)
        return (-bb+math.sqrt(disc))/(2*aa)
    if packet.get('irregular_source_fragments'):
        rng_front=np.random.default_rng(81700+packet['native_mask'])
        xs=np.linspace(0,width,math.ceil(width/3)+1);ys=np.linspace(0,height,math.ceil(height/3)+1)
        samples=[]
        for iy,sy in enumerate(ys):
            for ix,sx in enumerate(xs):
                samples.append([sx+(rng_front.uniform(-.7,.7) if 0<ix<len(xs)-1 else 0),
                                sy+(rng_front.uniform(-.7,.7) if 0<iy<len(ys)-1 else 0)])
        samples=np.asarray(samples)
        for face in Delaunay(samples).simplices:
            xy=samples[face];lo=np.floor(xy.min(axis=0)).astype(int);hi=np.ceil(xy.max(axis=0)).astype(int)
            if not alpha[max(0,lo[1]):min(height,hi[1]),max(0,lo[0]):min(width,hi[0])].any():continue
            sx,sy=xy.mean(axis=0)+[x0,y0]
            uneven=(5*math.sin(sx*.052+sy*.031)+3*math.sin(sx*.11-sy*.057))*min(1,fw/220)
            depth=front_depth(sx,sy)+rng_front.uniform(-.3,.3)
            pts=[point(x0+px,y0+py,depth) for px,py in xy]
            coords=[(px/width,1-py/height) for px,py in xy]
            if np.dot(np.cross(pts[1]-pts[0],pts[2]-pts[0]),ray)<0:
                pts.reverse();coords.reverse()
            for slot,offset,observed,reverse in [(0,0,True,False),(1,.001,False,False),(2,.02,False,True)]:
                start=len(vertices);vertices.extend([list(p-ray*offset) for p in pts]);uvs.extend(coords)
                faces.append((start+2,start+1,start) if reverse else (start,start+1,start+2))
                slots.append(slot);known.append(observed)
    else:raise ValueError('Conifer requires irregular source triangles')
    patches=[(x,y) for y in range(0,height-12,3) for x in range(0,width-12,3) if known_alpha[y:y+12,x:x+12].mean()>.25]
    if not patches:raise ValueError('No native leaf patches')
    rng=np.random.default_rng(55100+packet['native_mask']);tiles=[]
    sample_v,sample_u=np.mgrid[0:24,0:24]/24+.5/24
    cluster_count=int(packet.get('cluster_count',1600));atlas_rows=max(64,math.ceil(cluster_count*6/48))
    for _ in range(cluster_count):
        height_fraction=float(rng.uniform(.025,.995) if rng.random()<.25 else 1-rng.uniform(.001,.98)**(1/3))
        angle=float(rng.uniform(0,2*math.pi))
        radius=base_radius*(1-height_fraction)*float(rng.uniform(.1,1)**.5)
        pos=root+np.array([math.cos(angle)*radius,math.sin(angle)*radius,height_fraction*crown_height])
        axis=rng.normal(size=3);axis/=np.linalg.norm(axis)
        other=np.cross(axis,[0,0,1] if abs(axis[2])<.9 else [1,0,0]);other/=np.linalg.norm(other)
        third=np.cross(axis,other);size=float(rng.uniform(2.,4.2))*max(.2,1-height_fraction*.75)
        px,py=patches[int(rng.integers(len(patches)))]
        leaf=np.asarray(Image.fromarray(inferred[py:py+12,px:px+12]).resize((24,24),Image.Resampling.NEAREST)).copy()
        if packet.get('irregular_inferred_alpha'):
            gy,gx=np.mgrid[0:24,0:24];gx=(gx+.5)/12-1;gy=(gy+.5)/12-1
            angle=np.arctan2(gy,gx)
            radius=.66+.13*np.sin(3*angle+rng.uniform(0,2*math.pi))+.08*np.sin(5*angle+rng.uniform(0,2*math.pi))
            value=leaf[:,:,:3].astype(float)@[.2126,.7152,.0722]
            donor=leaf[:,:,3]>127
            threshold=float(np.quantile(value[donor],.20)) if donor.any() else 255
            # Native front alpha is never changed. Hidden donors need their
            # own irregular cluster silhouette, not an opaque crop rectangle.
            cut=(np.sqrt(gx*gx+gy*gy)<radius)&(value>=threshold)
            leaf[:,:,3]=np.where(cut,leaf[:,:,3],0)
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
    # A slender connected axis and tapered branch whorls support the inferred
    # needles. Their front is clipped to own native alpha; no borrowed bark.
    palette=visible[known_alpha][:,:3].astype(float)
    brown=(palette[:,0]>palette[:,1]*.93)&(palette[:,1]>palette[:,2]*1.1)
    wood_rgb=np.median(palette[brown] if brown.any() else palette,axis=0).astype('uint8')
    wood_front=np.zeros_like(rgba);wood_front[:,:,:3]=wood_rgb;wood_front[:,:,3]=rgba[:,:,3]
    Image.fromarray(wood_front).save(directory/'inferred-wood-front.png')
    Image.fromarray(np.array([[list(wood_rgb)+[255]]],dtype='uint8')).save(directory/'inferred-wood-back.png')
    mats.extend([material(obj.name+' inferred clipped native wood',directory/'inferred-wood-front.png',False),material(obj.name+' inferred native wood backs',directory/'inferred-wood-back.png',False)])
    one_sided(mats[5]);one_sided(mats[6])
    def branch(start,end,r0,r1):
        axis=end-start;axis/=np.linalg.norm(axis)
        u=np.cross(axis,[1,0,0] if abs(axis[2])>.9 else [0,0,1]);u/=np.linalg.norm(u);v=np.cross(axis,u)
        for face in range(7):
            a=2*math.pi*face/7;b=2*math.pi*(face+1)/7
            points=[start+r0*(u*math.cos(a)+v*math.sin(a)),start+r0*(u*math.cos(b)+v*math.sin(b)),end+r1*(u*math.cos(b)+v*math.sin(b)),end+r1*(u*math.cos(a)+v*math.sin(a))]
            if np.dot(np.cross(points[1]-points[0],points[2]-points[0]),ray)>0:
                coords=[((p[0]-x0)/width,1-(-p[1]*SIN-p[2]*COS-y0)/height) for p in points]
                quad(points,coords,5)
            else:quad(points,[(.5,.5)]*4,6)
    branch(root,root+np.array([0,0,crown_height*.985]),1.0,.06)
    branch_count=0
    for tier,t in enumerate(np.linspace(.12,.90,10)):
        length=base_radius*(1-t)*.91
        for spoke in range(7):
            angle=spoke*2*math.pi/7+tier*.47
            start=root+np.array([0,0,t*crown_height])
            end=start+np.array([math.cos(angle)*length,math.sin(angle)*length,-length*.13])
            branch(start,end,.45*(1-t)+.08,.045);branch_count+=1
    result=replace_mesh(obj,vertices,faces,uvs,mats,slots,known)
    result['inferred_connected_support']=dict(axis=True,tapered_branches=branch_count,material='Own native visible brown palette; no external bark donor')
    obj['projection_component']='crown';obj['projection_preserve']=True;obj['foliage_physical_opacity']=True
    bounds=measure(obj)
    for _ in range(3):
        if bounds['depth_width_ratio']>=1.02:break
        scale=1.04/bounds['depth_width_ratio']
        for vertex in obj.data.vertices:
            p=np.asarray(vertex.co);d=float(np.dot(p-center,ray));vertex.co=p+ray*d*(scale-1)
        obj.data.update();bounds=measure(obj)
    minimum=min(v.co.z for v in obj.data.vertices)
    minimum_target=float(packet.get('minimum_elevation',.5))
    if minimum<minimum_target:
        for v in obj.data.vertices:v.co+=RAY*((minimum_target-minimum)/SIN)
    result.update(geometry_version='native-conifer-leaf-volume-v1',native_mask=packet['native_mask'],
        source_projection_preserved=True,observed_leaf_pixels=int(known_alpha.sum()),inferred_covered_pixels=int((alpha&~known_alpha).sum()),
        leaf_clusters=len(tiles),opacity_bounds=measure(obj),minimum_z=min(v.co.z for v in obj.data.vertices),
        minimum_elevation_target=minimum_target,support_evidence=packet.get('support_evidence'),
        inferred_donor_alpha='irregular silhouette and luminance cut' if packet.get('irregular_inferred_alpha') else 'native crop alpha',
        source_fragment_layout='jittered Delaunay triangles' if packet.get('irregular_source_fragments') else 'regular source patches',
        method=('Own-native microtriangles on an upright conical envelope' if packet.get('irregular_source_fragments') else 'Small observed front cutouts on a round world volume')+'; source-clipped interior leaves and one-sided inferred rear volume',
        references=[])
    return result
