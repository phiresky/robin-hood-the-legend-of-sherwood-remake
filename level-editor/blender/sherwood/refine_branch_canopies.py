"""Tree-owned branch networks, small leaf sprays and locally mapped rear leaves.

Run through Blender MCP after the other Sherwood passes. TARGET_PROFILES can
select a pilot crown in a fresh output collection. The only measured reference
is the map and sprite alpha; concealed branching and leaf orientations are inferred.
"""
import json
import math
import random
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import ROOT,DATA,SIN,COS,EYE,collection,game_point

NAME='10 Animated foliage - curved canopy shells'
c=collection(NAME)
UP=Vector((0,SIN,COS));RIGHT=Vector((1,0,0))
records=json.loads((ROOT/'level-editor/work/sherwood-refinement/animation-references/manifest.json').read_text())['assets']
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())
refs=bpy.data.collections['07 Reference only - authored animated sprites']
supports={'Sherwood - Arbre01':[27,30,32], 'Sherwood - Arbre02':[46,35,29,31,24,25,27],
          'Sherwood - Arbre03':[44,42,45,36,37], 'Sherwood - Arbre04':[34],
          'Sherwood - Arbre05':[-5], 'Sherwood - Arbre06':[40]}
targets=globals().get('TARGET_PROFILES')
report={'trees':[],'inference':'Crown ownership follows supporting trunks; concealed branches and leaf orientations inferred'}
fork_heights={}
fit_path=Path(__file__).with_name('foliage_fit_samples.json')
fit_samples=json.loads(fit_path.read_text()) if fit_path.is_file() else {}

def root(index):
    if index==-5:return (445.,1120.)
    ps=level['sight_obstacles'][index]['points']
    return ((min(p['x'] for p in ps)+max(p['x'] for p in ps))/2,
            (min(p['y'] for p in ps)+max(p['y'] for p in ps))/2)

def atlas_material(ref,uv_name,label):
    m=ref.data.materials[0].copy();m.name=ref['profile']+' - '+label
    nodes=m.node_tree.nodes;links=m.node_tree.links
    old=next(n for n in nodes if n.type=='TEX_COORD')
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Source canvas'
    for link in list(links):
        if link.from_socket==old.outputs['UV']:links.new(uv.outputs['UV'],link.to_socket)
    split=nodes.new('ShaderNodeSeparateXYZ');links.new(uv.outputs['UV'],split.inputs[0])
    mask=None
    for axis in ['X','Y']:
        for op,limit in [('GREATER_THAN',0),('LESS_THAN',1)]:
            n=nodes.new('ShaderNodeMath');n.operation=op;n.inputs[1].default_value=limit
            links.new(split.outputs[axis],n.inputs[0])
            if mask is None:mask=n.outputs[0]
            else:
                p=nodes.new('ShaderNodeMath');p.operation='MULTIPLY'
                links.new(mask,p.inputs[0]);links.new(n.outputs[0],p.inputs[1]);mask=p.outputs[0]
    tex=next(n for n in nodes if n.type=='TEX_IMAGE')
    p=nodes.new('ShaderNodeMath');p.operation='MULTIPLY'
    links.new(mask,p.inputs[0]);links.new(tex.outputs['Alpha'],p.inputs[1])
    links.new(p.outputs[0],next(n for n in nodes if n.type=='MIX_SHADER').inputs[0])
    if uv_name=='Leaf surface':
        # Keep the authored silhouette/holes on both sides. Only rear COLOR
        # uses leaf-local UVs, so local samples cannot fill reference openings.
        r=next(r for r in records if r['profile']==ref['profile'])
        w,h=r['canvas'];aw,ah=tex.image.size;g=r['gutter']
        local=nodes.new('ShaderNodeUVMap');local.uv_map='Leaf surface'
        scale=nodes.new('ShaderNodeVectorMath');scale.operation='MULTIPLY'
        scale.inputs[1].default_value=(w/aw,h/ah,1)
        add=nodes.new('ShaderNodeVectorMath');add.operation='ADD'
        links.new(local.outputs['UV'],scale.inputs[0]);links.new(scale.outputs[0],add.inputs[0])
        index='(floor((frame-1)/4)%16)'
        expressions=[f'(({index}%4)*{w+2*g}+{g})/{aw}',f'((3-floor({index}/4))*{h+2*g}+{g})/{ah}']
        for axis,expression in enumerate(expressions):
            driver=add.inputs[1].driver_add('default_value',axis).driver
            driver.type='SCRIPTED';driver.expression=expression
        sample=nodes.new('ShaderNodeTexImage');sample.image=tex.image;sample.interpolation='Closest';sample.extension='CLIP'
        links.new(add.outputs[0],sample.inputs['Vector'])
        links.new(sample.outputs['Color'],next(n for n in nodes if n.type=='EMISSION').inputs['Color'])
    return m

class Batch:
    def __init__(self):self.vertices=[];self.faces=[];self.local_uv=[]
    def add(self,verts,faces,uvs=None):
        start=len(self.vertices);self.vertices.extend(verts)
        self.faces.extend([tuple(start+i for i in f) for f in faces])
        self.local_uv.extend(uvs if uvs is not None else [(0,0)]*len(verts))
    def tube(self,points,radii,sides=7):
        vertices=[];coords=[];distance=0
        for i,(p,r) in enumerate(zip(points,radii)):
            if i:distance+=(points[i]-points[i-1]).length
            tangent=(points[min(i+1,len(points)-1)]-points[max(0,i-1)]).normalized()
            n=tangent.cross(UP)
            if n.length<.01:n=tangent.cross(RIGHT)
            n.normalize();b=tangent.cross(n).normalized()
            vertices.extend([p+r*(n*math.cos(j*math.tau/sides)+b*math.sin(j*math.tau/sides)) for j in range(sides)])
            coords.extend([(j/sides,distance/95) for j in range(sides)])
        faces=[(i*sides+j,i*sides+(j+1)%sides,(i+1)*sides+(j+1)%sides,(i+1)*sides+j) for i in range(len(points)-1) for j in range(sides)]
        faces.extend([tuple(reversed(range(sides))),tuple((len(points)-1)*sides+j for j in range(sides))])
        self.add(vertices,faces,coords)
    def object(self,name,materials,record=None,left=0,top=0):
        data=bpy.data.meshes.new(name);data.from_pydata(self.vertices,[],self.faces);data.update()
        obj=bpy.data.objects.new(name,data);c.objects.link(obj)
        for m in materials:data.materials.append(m)
        uv=data.uv_layers.new(name='Source canvas' if record else 'Bark')
        local=data.uv_layers.new(name='Leaf surface') if record else None
        for loop in data.loops:
            v=data.vertices[loop.vertex_index].co
            if record:
                uv.data[loop.index].uv=((v.x-left)/record['canvas'][0],1-(-v.y*SIN-v.z*COS-top)/record['canvas'][1])
                local.data[loop.index].uv=self.local_uv[loop.vertex_index]
            else:uv.data[loop.index].uv=self.local_uv[loop.vertex_index]
        for face in data.polygons:
            face.use_smooth=not record
            if record:face.material_index=0 if face.normal.dot(EYE)>.18 else 1
            elif len(face.vertices)==4:
                values=[uv.data[i].uv.x for i in face.loop_indices]
                if max(values)-min(values)>.5:
                    for i in face.loop_indices:
                        if uv.data[i].uv.x<.5:uv.data[i].uv.x+=1
        return obj

def groups(points,k):
    """Deterministic farthest-seeded clustering for branch leaders and forks."""
    a=np.array([tuple(p) for p in points]);k=min(k,len(a))
    centers=[a[len(a)//2]]
    for _ in range(1,k):
        distances=np.min(np.sum((a[:,None,:]-np.array(centers)[None,:,:])**2,axis=2),axis=1)
        centers.append(a[int(np.argmax(distances))])
    centers=np.array(centers)
    for _ in range(8):
        labels=np.argmin(np.sum((a[:,None,:]-centers[None,:,:])**2,axis=2),axis=1)
        for i in range(k):
            if np.any(labels==i):centers[i]=a[labels==i].mean(axis=0)
    return [[int(i) for i in np.flatnonzero(labels==j)] for j in range(k) if np.any(labels==j)]

def trim_trunk(index,z):
    if index in [-5,32]:return
    candidates=[o for o in bpy.data.collections['08 Refined forest - trunks roots and branches'].objects
                if o.get('source_obstacle')==f'building-{index:03}' and 'tapered trunk' in o.name]
    if index==24:candidates=[bpy.data.objects['Ladder oak - tapered fluted trunk']]
    if not candidates:raise RuntimeError(f'Missing supporting trunk {index}')
    obj=candidates[0]
    if max(v.co.z for v in obj.data.vertices)<=z+2:return
    # A native backup is saved before this pass. Keep the cropped stem editable.
    bm=bmesh.new();bm.from_mesh(obj.data)
    cut=bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),
        plane_co=Vector((0,0,z)),plane_no=Vector((0,0,1)),clear_outer=True,clear_inner=False,dist=.0001)
    edges=[e for e in cut['geom_cut'] if isinstance(e,bmesh.types.BMEdge) and e.is_boundary]
    if edges:
        caps=bmesh.ops.holes_fill(bm,edges=edges,sides=0)['faces']
        uv=bm.loops.layers.uv.active
        donor=next((i for i,m in enumerate(obj.data.materials) if m.name.startswith('Forest - concealed bark')),None)
        if donor is None:raise RuntimeError(f'Missing concealed bark material for trunk {index}')
        cx,gy=root(index)
        for face in caps:
            face.material_index=donor
            for loop in face.loops:loop[uv].uv=((loop.vert.co.x-cx)/60+.5,(loop.vert.co.y+gy/SIN)/60+.5)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free()
    obj['crown_fork_height_world']=z

for record in records:
    if record['kind']!='tree' or (targets and record['profile'] not in targets):continue
    profile=record['profile'];rng=random.Random(profile+' branch-grown')
    ref=next(o for o in refs.objects if o.get('profile')==profile)
    front=atlas_material(ref,'Source canvas','projected front leaves')
    back=atlas_material(ref,'Leaf surface','locally mapped side and rear leaves')
    image=next(n.image for n in ref.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE')
    aw,ah=image.size;buf=np.empty(aw*ah*4,dtype=np.float32);image.pixels.foreach_get(buf)
    alpha=buf.reshape(ah,aw,4)[:,:,3];w,h=record['canvas'];g=record['gutter']
    frames=np.stack([alpha[row*(h+2*g)+g:row*(h+2*g)+g+h,col*(w+2*g)+g:col*(w+2*g)+g+w][::-1]
                     for row in range(4) for col in range(4)])
    union=frames.max(axis=0)>.01;stable=frames.min(axis=0)>.95
    safe=stable.copy()
    for dy in range(-3,4):
        for dx in range(-3,4):safe &= np.roll(np.roll(stable,dy,axis=0),dx,axis=1)
    safe[:4]=False;safe[-4:]=False;safe[:,:4]=False;safe[:,-4:]=False
    donors=np.argwhere(safe)
    if not len(donors):raise RuntimeError(f'No stable opaque leaf patch for {profile}')
    p0=ref.matrix_world@ref.data.vertices[0].co;left=p0.x;top=-p0.y*SIN-p0.z*COS
    owners={i:[] for i in supports[profile]}
    step=7
    # Source occupancy guides sprays and preserves the observed holes. A sample
    # is assigned to a real trunk, never a continuously interpolated sheet.
    for y in range(0,h,step):
        for x in range(0,w,step):
            block=union[y:min(y+step,h),x:min(x+step,w)]
            active=np.argwhere(block)
            if not len(active):continue
            sy,sx=active[rng.randrange(len(active))];px=left+x+int(sx);py=top+y+int(sy)
            owner=min(supports[profile],key=lambda i:(root(i)[0]-px)**2+max(0,py+30-root(i)[1])**2*30)
            owners[owner].append((px,py))
    for px,py in fit_samples.get(profile,[]):
        owner=min(supports[profile],key=lambda i:(root(i)[0]-px)**2+max(0,py+30-root(i)[1])**2*30)
        owners[owner].append((float(px),float(py)))
    for owner,samples in owners.items():
        if not samples:continue
        cx,gy=root(owner);pixels=np.array(samples);mean=pixels.mean(axis=0)
        half=np.maximum(np.ptp(pixels,axis=0)*.5,20)
        center=game_point(float(mean[0]),gy,gy-float(mean[1]))
        # Build around the supporting tree's world-vertical axis. The old
        # camera-plane ellipsoid used min(half), making broad, shallow source
        # silhouettes into tilted slabs. Horizontal crown spread supplies depth.
        crown_radius_y=max(35.,float(half[0])*1.08)
        points=[]
        for px,py in samples:
            point_rng=random.Random(f'{profile}:{owner}:{px:.3f}:{py:.3f}:depth')
            vertical=mean[1]-py
            radial=((px-mean[0])/(half[0]*1.1))**2+.25*(vertical/(half[1]*1.1))**2
            depth=vertical*SIN/COS+crown_radius_y/COS*math.sqrt(max(.08,1-radial))*point_rng.uniform(-1,1)
            points.append(center+RIGHT*(px-mean[0])+UP*(mean[1]-py)+EYE*depth)
        minimum={24:357,29:327,30:337,46:353,34:92}.get(owner,35)
        fork_height=max(minimum,(gy-float(np.percentile(pixels[:,1],85)))*.75)
        origin=game_point(cx,gy,fork_height)
        if owner==32:origin=game_point(1021,563,380)
        # Keep lower leaf sprays clear of the supporting stem. Moving along a
        # reference-camera ray changes depth without moving the observed outline.
        if owner==-5:clearance=17
        else:
            ps=level['sight_obstacles'][owner]['points']
            clearance=(max(p['x'] for p in ps)-min(p['x'] for p in ps))*.43+7
        for point in points:
            dx=point.x-cx;dy=point.y+gy/SIN
            if abs(dx)<clearance and point.z<origin.z+65:
                limit=math.sqrt(clearance*clearance-dx*dx)
                if abs(dy)<limit:
                    desired=limit if dy>=0 else -limit
                    point+=EYE*((dy-desired)/COS)
            if point.z<12:
                point+=EYE*((12-point.z)/SIN)
        fork_heights[owner]=max(fork_heights.get(owner,0),origin.z)
        bark=next(m for m in bpy.data.materials if m.name.startswith('Forest - concealed bark from source oak'))
        branches=Batch();leaves=Batch();cores=Batch()
        if owner==-5:
            branches.tube([game_point(cx,gy,0),game_point(cx-2,gy,fork_height*.65),origin],[10,7,4],12)
        elif owner!=32:
            if owner==24:stem=bpy.data.objects['Ladder oak - tapered fluted trunk']
            else:stem=next(o for o in bpy.data.collections['08 Refined forest - trunks roots and branches'].objects
                          if o.get('source_obstacle')==f'building-{owner:03}' and 'tapered trunk' in o.name)
            top_z=max(v.co.z for v in stem.data.vertices)
            if top_z<origin.z-.1:
                rim=[v.co for v in stem.data.vertices if v.co.z>top_z-.01]
                foot=sum(rim,Vector())/len(rim)
                radius=sum((v-foot).length for v in rim)/len(rim)
                branches.tube([foot,foot.lerp(origin,.5),origin],[radius,radius*.86,radius*.7],12)
        majors=groups(points,min(7,max(3,len(points)//160)))
        spray_parents={};branch_count=0
        for mi,ids in enumerate(majors):
            end=sum((points[i] for i in ids),Vector())/len(ids)
            elbow=origin.lerp(end,.52)+Vector((0,0,12))
            radius=min(8,max(2.1,math.sqrt(len(ids))*.43))
            branches.tube([origin,elbow,end],[radius,radius*.64,1.35],9);branch_count+=1
            for sub in groups([points[i] for i in ids],min(6,max(2,len(ids)//35))):
                actual=[ids[i] for i in sub]
                tip=sum((points[i] for i in actual),Vector())/len(actual)
                junction=elbow.lerp(end,.55)
                branches.tube([junction,end.lerp(tip,.45),tip],[1.5,1,.45],7);branch_count+=1
                for tertiary in groups([points[i] for i in actual],max(1,len(actual)//9)):
                    terminal=[actual[i] for i in tertiary]
                    bud=sum((points[i] for i in terminal),Vector())/len(terminal)
                    branches.tube([tip,tip.lerp(bud,.60)+Vector((0,0,1.5)),bud],[.65,.4,.18],6)
                    branch_count+=1
                    for i in terminal:spray_parents[i]=bud
        for i,(p,(px,py)) in enumerate(zip(points,samples)):
            rng=random.Random(f'{profile}:{owner}:{px:.3f}:{py:.3f}:leaves')
            parent=spray_parents[i]
            axis=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(.2,1))).normalized()
            stem_start=p-axis*5;stem_end=p+axis*5
            # Fine terminal twigs attach every spray to its nearest fork.
            near,far=sorted([stem_start,stem_end],key=lambda q:(q-parent).length_squared)
            if (parent-near).length<.0001:
                branches.tube([near,far],[.19,.08],5)
            else:
                branches.tube([parent,parent.lerp(near,.72),near,far],[.48,.3,.19,.08],5)
            # Sample a dense patch locally for reverse sides; front faces keep
            # the original whole-canvas projection and animated alpha.
            donor_rng=random.Random(f'{profile}:{owner}:{px:.3f}:{py:.3f}:donor')
            candidates=donors[[rng.randrange(len(donors))]+[donor_rng.randrange(len(donors)) for _ in range(23)]]
            distances=(candidates[:,1]-(px-left))**2+(candidates[:,0]-(py-top))**2
            candidate=candidates[int(np.argmin(distances))];donor_y,donor_x=map(float,candidate)
            for j in range(9):
                t=(j+.5)/9
                # Varied world-space leaf inclinations, without a shared
                # reference-camera normal that turns sprays edge-on together.
                normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.3,1))).normalized()
                longitudinal=axis-normal*axis.dot(normal)
                if longitudinal.length<.05:longitudinal=normal.cross(RIGHT)
                longitudinal.normalize();lateral=normal.cross(longitudinal).normalized()
                if j%2:longitudinal=-longitudinal
                length=rng.uniform(3.4,6.0);width=rng.uniform(1.4,2.8)
                pos=stem_start.lerp(stem_end,t)+lateral*((-1 if j%2 else 1)*rng.uniform(2,5))
                outline=[(-1,0),(-.48,.78),(.25,1),(1,0),(.25,-1),(-.48,-.78)]
                verts=[pos+longitudinal*(u*length)+lateral*(v*width) for u,v in outline]
                verts.extend([pos+normal*.42,pos-normal*.12])
                faces=[((k+1)%6,k,6) for k in range(6)]+[(k,(k+1)%6,7) for k in range(6)]
                # Leaf-local UVs remain well scaled from the side and rear.
                coords=[((donor_x+u*2.5)/w,1-(donor_y+v*2.5)/h) for u,v in outline]+[(donor_x/w,1-donor_y/h)]*2
                leaves.add(verts,faces,coords)
        # Just two modest inner masses support density; the outline and visible
        # outer crown are formed by small leaf sprays, not bumpy spheres.
        for mass in range(min(2,len(majors))):
            ids=majors[mass];p=sum((points[i] for i in ids),Vector())/len(ids)
            bm=bmesh.new();bmesh.ops.create_icosphere(bm,subdivisions=2,radius=1)
            bm.verts.ensure_lookup_table();bm.verts.index_update()
            donor_y,donor_x=map(float,donors[rng.randrange(len(donors))])
            verts=[p+Vector((v.co.x*min(half[0]*.20,28),v.co.y*min(crown_radius_y*.20,28),v.co.z*20)) for v in bm.verts]
            coords=[((donor_x+v.co.x*2.5)/w,1-(donor_y+v.co.y*2.5)/h) for v in bm.verts]
            cores.add(verts,[[v.index for v in f.verts] for f in bm.faces],coords);bm.free()
        label=f'{profile} tree {owner:03}'
        branch_obj=branches.object(label+' - limbs forks and terminal twigs',[bark])
        leaf_obj=leaves.object(label+' - individual leaf sprays',[front,back],record,left,top)
        core_obj=cores.object(label+' - inner foliage masses',[front,back],record,left,top)
        for obj in [branch_obj,leaf_obj,core_obj]:
            obj['profile']=profile;obj['supporting_tree']=owner
            obj['inferred']='Branch layout, leaf normals and concealed depth from single-view reconstruction'
        leaf_obj['sprays']=len(points);leaf_obj['individual_leaves']=len(points)*9
        leaf_obj['crown_radius_y']=crown_radius_y
        leaf_obj['volume_method']='World-vertical crown; horizontal spread sets depth; projection-preserving ray placement'
        report['trees'].append({'profile':profile,'trunk':owner,'sprays':len(points),'leaves':len(points)*9,
            'main_and_secondary_branches':branch_count,'fork_height_world':origin.z})
for owner,height in fork_heights.items():trim_trunk(owner,height)
result=report
