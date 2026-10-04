"""Source-traced wood and full-depth fixed foliage for Croisement02.

Foliage construction follows the two selected Leicester cottage/moat trees:
separate wood and overlapping fixed cutouts with transverse leaf surfaces.
All RGB comes from this map. Back/side RGB is an explicit source-derived fill,
not observed rear artwork. No other tree model is used as a reference.
"""
import json
import math
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from PIL import Image
from catalog import OUT
SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
RAY=Vector((0,-COS,SIN))


def material(name,path,known=True):
    mat=bpy.data.materials.new(name);mat.use_nodes=True
    mat.use_backface_culling=False
    if hasattr(mat,'surface_render_method'):mat.surface_render_method='DITHERED'
    mat['foliage_physical_opacity']=True;mat['foliage_alpha_cutoff']=.5
    mat['opacity_semantics']='physical-coverage'
    mat['projection_preserve']=True;mat['source_ownership_semantics']='separate-mask'
    mat['source_ownership_channel']='vertex-color-r';mat['foliage_observed']=known
    mat['texture_provenance']='observed front' if known else 'inferred rear/side using same Croisement02 foliage'
    nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Foliage UV'
    tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path),check_existing=False);tex.image.pack();tex.interpolation='Closest'
    shader=nodes.new('ShaderNodeBsdfPrincipled');shader.inputs['Roughness'].default_value=1
    output=nodes.new('ShaderNodeOutputMaterial');links.new(uv.outputs['UV'],tex.inputs['Vector'])
    links.new(tex.outputs['Color'],shader.inputs['Base Color']);links.new(tex.outputs['Alpha'],shader.inputs['Alpha'])
    links.new(tex.outputs['Color'],shader.inputs['Emission Color']);shader.inputs['Emission Strength'].default_value=1
    links.new(shader.outputs[0],output.inputs[0]);return mat


def one_sided(mat):
    """Match explicit Cycles culling to the reference models' raster flag."""
    mat.use_backface_culling=True;mat['foliage_card_sides']='paired-one-sided'
    nodes=mat.node_tree.nodes;links=mat.node_tree.links
    if nodes.get('One-sided foliage'):return
    output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL')
    original=output.inputs['Surface'].links[0].from_socket
    geometry=nodes.new('ShaderNodeNewGeometry');transparent=nodes.new('ShaderNodeBsdfTransparent')
    mix=nodes.new('ShaderNodeMixShader');mix.name='One-sided foliage'
    links.new(geometry.outputs['Backfacing'],mix.inputs[0]);links.new(original,mix.inputs[1]);links.new(transparent.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],output.inputs['Surface'])


def replace_mesh(obj,vertices,faces,uvs=None,materials=(),slots=None,known=None):
    mesh=bpy.data.meshes.new(obj.name+' refined');inverse=obj.matrix_world.inverted()
    mesh.from_pydata([inverse@Vector(p) for p in vertices],[],faces);mesh.update()
    for mat in materials:mesh.materials.append(mat)
    uv=mesh.uv_layers.new(name='Foliage UV')
    ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER')
    mesh.color_attributes.active_color=ownership
    for face in mesh.polygons:
        if slots is not None:face.material_index=slots[face.index]
        for loop in face.loop_indices:
            vertex=mesh.loops[loop].vertex_index
            uv.data[loop].uv=uvs[vertex] if uvs else (vertices[vertex][0]/1792,1-(-vertices[vertex][1]*SIN-vertices[vertex][2]*COS)/1152)
            ownership.data[loop].color=(float(known[face.index]) if known else 0.,1,1,1)
    bm=bmesh.new();bm.from_mesh(mesh)
    degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
    boundary=sum(e.is_boundary for e in bm.edges);nonmanifold=sum(not e.is_manifold for e in bm.edges)
    if degenerate:raise ValueError(f'{obj.name}: {degenerate} degenerate faces')
    if not uvs:bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bm.to_mesh(mesh);bm.free();mesh.update();obj.data=mesh
    return dict(vertices=len(vertices),faces=len(faces),boundary_edges=boundary,nonmanifold_edges=nonmanifold,degenerate_faces=degenerate)


def wood_geometry(paths,ground_y):
    vertices=[];faces=[]
    for path in paths:
        if len(path)<2:continue
        centers=[Vector((x,-ground_y/SIN,(ground_y-y)/COS)) for x,y,r in path]
        start=len(vertices);n=12
        for i,(center,point) in enumerate(zip(centers,path)):
            axis=centers[min(i+1,len(centers)-1)]-centers[max(0,i-1)]
            if axis.length<1e-6:raise ValueError('Degenerate wood centreline')
            axis.normalize();side=axis.cross(RAY).normalized();up=axis.cross(side).normalized()
            for j in range(n):
                angle=j*math.tau/n;radius=point[2]*(1+.035*math.cos(angle*5+i*.2))
                vertices.append(tuple(center+radius*(side*math.cos(angle)+up*math.sin(angle))))
        faces.append(tuple(start+j for j in reversed(range(n))))
        for i in range(len(path)-1):
            for j in range(n):faces.append((start+i*n+j,start+i*n+(j+1)%n,start+(i+1)*n+(j+1)%n,start+(i+1)*n+j))
        faces.append(tuple(start+(len(path)-1)*n+j for j in range(n)))
    return vertices,faces


def foliage_packet(animation,seeds,destination,sector=None):
    data=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'][animation]
    frame=data['frames'][0]
    inventory=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    native=next(r for r in inventory['masks'] if r['index']==128+animation)
    x,y=native['box_top_left'];w,h=native['box_size']
    alpha=np.asarray(Image.open(OUT/'baseline/masks'/native['png']).convert('L'))>0
    rgb=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop((x,y,x+w,y+h)))
    rgba=np.zeros((h,w,4),dtype=np.uint8);rgba[:,:,:3]=rgb
    destination.mkdir(parents=True,exist_ok=True)
    if sector is not None:
        sy,sx=np.mgrid[:h,:w];sx=sx+x;sy=sy+y
        selected,centres=sector
        distances=np.stack([(sx-cx)**2+(sy-cy)**2 for _,cx,cy in centres],axis=2)
        nearest=alpha&(np.argmin(distances,axis=2)==selected)
        if not nearest.any():raise ValueError('Empty crown ownership seed')
        # Overlap rounded supports; do not expose straight Voronoi borders.
        # RGB in overlaps uses identical source coordinates. Individual tree
        # ownership inside a shared native canopy remains inferred.
        radius2=float(distances[:,:,selected][nearest].max())*1.10**2
        alpha &= distances[:,:,selected]<=radius2
    rgba[:,:,3]=alpha*255
    Image.fromarray(rgba).save(destination/'complete-source.png')
    yy,xx=np.mgrid[:h,:w];sx,sy=xx+x,yy+y
    points=np.asarray(seeds,dtype=float)
    samples=np.column_stack((sx[alpha],sy[alpha]))[::3]
    # Fit the eight selected-reference lobe centres to this canopy's leaf
    # masses. This does not assign ownership between separate trees.
    for _ in range(12):
        assignment=np.argmin(np.sum((samples[:,None,:]-points[None,:,:])**2,axis=2),axis=1)
        for k in range(len(points)):
            if np.any(assignment==k):points[k]=np.mean(samples[assignment==k],axis=0)
    seeds=points.tolist()
    dist=(sx[:,:,None]-points[:,0])**2+(sy[:,:,None]-points[:,1])**2
    labels=np.argmin(dist,axis=2);destination.mkdir(parents=True,exist_ok=True);records=[];union=np.zeros_like(alpha)
    for i,(cx,cy) in enumerate(seeds):
        nearest=alpha&(labels==i)
        if not nearest.any():continue
        radius=math.sqrt(float(dist[:,:,i][nearest].max()))*1.15
        support=alpha&(dist[:,:,i]<=radius*radius)
        # Keep the complete circular support so opposing hemispheres meet at
        # their equator instead of ending in disconnected curved sheets.
        x0,y0=math.floor(cx-radius),math.floor(cy-radius)
        x1,y1=math.ceil(cx+radius),math.ceil(cy+radius)
        pixels=np.zeros((y1-y0,x1-x0,4),dtype=np.uint8)
        left,top=max(x,x0),max(y,y0);right,bottom=min(x+w,x1),min(y+h,y1)
        pixels[top-y0:bottom-y0,left-x0:right-x0,:3]=rgb[top-y:bottom-y,left-x:right-x]
        pixels[top-y0:bottom-y0,left-x0:right-x0,3]=support[top-y:bottom-y,left-x:right-x]*255
        path=destination/f'lobe-{i:02}.png';Image.fromarray(pixels).save(path);union|=support
        records.append(dict(image=str(path),bbox=[x0,y0,x1,y1],seed=[cx,cy]))
    assert np.array_equal(union,alpha)
    ay,ax=np.nonzero(alpha)
    if not len(ax):raise ValueError('Empty canopy sector')
    source_bbox=[x+int(ax.min()),y+int(ay.min()),int(np.ptp(ax))+1,int(np.ptp(ay))+1]
    report=dict(animation=animation,profile=data['profile'],bbox=source_bbox,lobes=records,source_pixels=int(alpha.sum()),coverage_complete=True,native_mask=128+animation,native_bbox=native['box_top_left']+native['box_size'],coverage_provenance='Native occupancy-mask support with composited source RGB; not animated sprite alpha. Static leaf support requires visual review; rounded per-tree ownership is inferred')
    (destination/'partition.json').write_text(json.dumps(report,indent=2)+'\n');return report


def crown_geometry(obj,packet,ground_y,refined=True,depth_ratio=1.08):
    if refined:return leaf_cluster_geometry(obj,packet,ground_y,depth_ratio)
    # Low-resolution context proxy used only to freeze worker cameras and
    # surrounding occluders. Final crowns use the small leaf clusters below.
    vertices=[];faces=[];uvs=[];slots=[];ownership=[];materials=[]
    full_x,full_y,full_w,full_h=packet['bbox'];center_y=full_y+full_h/2
    for number,lobe in enumerate(packet['lobes']):
        x0,y0,x1,y1=lobe['bbox'];front_slot=len(materials)
        materials.append(material(f'{obj.name} lobe {number} front',lobe['image'],True))
        materials.append(material(f'{obj.name} lobe {number} inferred back',lobe['image'],False))
        start=len(vertices);rings=4;segments=8
        rx=(x1-x0)/2;ry=(y1-y0)/2;cx=(x1+x0)/2;cy=(y1+y0)/2
        radius=max(rx,ry);offset=[.5,-.5,.3,-.4,0.,.45,-.25,.25][number%8]*full_w*.65
        def add(px,py,pd):
            x=cx+rx*px;y=cy+ry*py
            point=Vector((x,-ground_y/SIN-(y-center_y)*SIN,(ground_y-center_y)/COS-(y-center_y)*COS))+RAY*(offset+radius*pd)
            vertices.append(tuple(point));uvs.append(((px+1)/2,(1-py)/2))
        add(0,-1,0)
        for j in range(1,rings):
            lat=-math.pi/2+math.pi*j/rings
            for i in range(segments):
                theta=math.tau*i/segments
                add(math.cos(lat)*math.cos(theta),math.sin(lat),math.cos(lat)*math.sin(theta))
        north=len(vertices);add(0,1,0)
        def face(indices,i):
            known=math.sin(math.tau*(i+.5)/segments)>0
            faces.append(indices);slots.append(front_slot+(0 if known else 1));ownership.append(known)
        for i in range(segments):
            k=(i+1)%segments
            face((start,start+1+k,start+1+i),i)
            for j in range(rings-2):
                aa=start+1+j*segments+i;bb=start+1+j*segments+k
                face((aa,bb,bb+segments,aa+segments),i)
            face((start+1+(rings-2)*segments+i,start+1+(rings-2)*segments+k,north),i)
    points=np.asarray(vertices);report=replace_mesh(obj,vertices,faces,uvs,materials,slots,ownership)
    obj['projection_component']='crown';obj['foliage_physical_opacity']=True;obj['projection_preserve']=True
    report.update(width=float(np.ptp(points[:,0])),depth=float(np.ptp(points[:,1])),source_projection_preserved=True,
                  rear_texture='Inferred reuse of this canopy RGB; no gray material placeholders',tree_references=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'])
    return report


def leaf_cluster_geometry(obj,packet,ground_y,depth_ratio=1.):
    """Distribute small fixed leaf cutouts through a rounded crown volume.

    Uses the selected references' separate wood and crossed foliage surfaces.
    Smaller source patches avoid stretching a leaf image around a large shell.
    Every source patch remains fixed in world space; nothing follows the camera.
    """
    path=Path(packet['lobes'][0]['image']).parent/'complete-source.png'
    source=np.asarray(Image.open(path).convert('RGBA'));alpha=source[:,:,3]>127
    x0,y0,w,h=packet['native_bbox'];fx,fy,fw,fh=packet['bbox'];center_y=fy+fh/2
    materials=[material(obj.name+' observed leaf patches',path,True),material(obj.name+' inferred crossed leaf patches',path,False),material(obj.name+' inferred leaf backs',path,False)]
    one_sided(materials[0]);one_sided(materials[2])
    vertices=[];faces=[];uvs=[];slots=[];ownership=[];cell=16
    rng=np.random.default_rng(int(packet["native_mask"])*7919+int(ground_y)*31)
    def point(x,y,d):
        return Vector((x,-ground_y/SIN-(y-center_y)*SIN,(ground_y-center_y)/COS-(y-center_y)*COS))+RAY*d
    def quad(points,coords,known):
        first=len(vertices);vertices.extend(tuple(p) for p in points);uvs.extend(coords)
        faces.extend([(first+2,first+1,first),(first+3,first+2,first)] if known else [(first,first+1,first+2),(first,first+2,first+3)])
        slots.extend([0 if known else 1]*2);ownership.extend([known]*2)
        if known:
            rear=len(vertices);vertices.extend(tuple(p-RAY*.02) for p in points);uvs.extend(coords)
            faces.extend([(rear,rear+1,rear+2),(rear,rear+2,rear+3)]);slots.extend([2]*2);ownership.extend([False]*2)
    count=0
    for top in range(0,h,cell):
        for left in range(0,w,cell):
            right,bottom=min(w,left+cell),min(h,top+cell)
            if not alpha[top:bottom,left:right].any():continue
            la,ra=max(0,left-4),min(w,right+4);ta,ba=max(0,top-4),min(h,bottom+4)
            xa,xb=x0+la,x0+ra;ya,yb=y0+ta,y0+ba;cx,cy=(xa+xb)/2,(ya+yb)/2
            radial=((cx-(fx+fw/2))/(fw*.6))**2+((cy-center_y)/(max(fh,1)*.65))**2
            available=math.sqrt(max(.12,1-radial))*fw*.58/COS
            uv=[(la/w,1-ta/h),(ra/w,1-ta/h),(ra/w,1-ba/h),(la/w,1-ba/h)]
            for layer in range(3):
                phase=float(rng.random())
                depth=(phase*2-1)*available
                # Keep low foliage above the ground without moving its source
                # projection; hidden vertical placement remains inferred.
                depth=max(depth,(4-(ground_y-center_y)/COS+(cy-center_y)*COS)/SIN)
                quad([point(xa,ya,depth),point(xb,ya,depth),point(xb,yb,depth),point(xa,yb,depth)],uv,True)
                radius=(xb-xa)*.6
                side_x=cx+float(rng.uniform(-.35,.35))*cell;side_y=cy+float(rng.uniform(-.35,.35))*cell
                quad([point(side_x,ya,depth-radius),point(side_x,ya,depth+radius),point(side_x,yb,depth+radius),point(side_x,yb,depth-radius)],uv,False)
                quad([point(xa,side_y,depth-radius),point(xb,side_y,depth-radius),point(xb,side_y,depth+radius),point(xa,side_y,depth+radius)],uv,False)
                count+=1
    pts=np.asarray(vertices);width=float(np.ptp(pts[:,0]));screen_y=-pts[:,1]*SIN-pts[:,2]*COS
    plane_y=-ground_y/SIN-(screen_y-center_y)*SIN;depths=(plane_y-pts[:,1])/COS
    low,high=0.,4.
    while np.ptp(plane_y-depths*high*COS)<width*depth_ratio:high*=2
    for _ in range(40):
        scale=(low+high)/2
        if np.ptp(plane_y-depths*scale*COS)<width*depth_ratio:low=scale
        else:high=scale
    pts+=np.asarray(RAY)[None,:]*depths[:,None]*((low+high)/2-1)
    if pts[:,2].min()<1:pts+=np.asarray(RAY)[None,:]*((1-pts[:,2].min())/SIN)
    report=replace_mesh(obj,pts.tolist(),faces,uvs,materials,slots,ownership)
    obj['projection_component']='crown';obj['foliage_physical_opacity']=True;obj['projection_preserve']=True
    report.update(width=float(np.ptp(pts[:,0])),depth=float(np.ptp(pts[:,1])),minimum_z=float(pts[:,2].min()),
                  geometry_version='native-leaf-clusters-v5',leaf_clusters=count,source_projection_preserved=True,
                  rear_texture='Inferred crossed surfaces reuse local leaf patches; source-only ownership remains separate',
                  tree_references=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'])
    return report
