"""Small native fragments and compact own-source interior leaves for reveal shrubs."""
import json,math,sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from scipy.ndimage import maximum_filter,gaussian_filter

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY,material,one_sided,replace_mesh


def main():
    audit=OUT/'restart14-hidden-archer/audit-v1'
    root=OUT/'restart14-hidden-archer/candidate-v2';root.mkdir(exist_ok=False)
    states=json.loads((audit/'source-authority.json').read_text())['profiles']
    measured=json.loads((audit/'substrate-first-hit-v1/report.json').read_text())['profiles']
    ray=np.array(RAY)
    for profile in states:
        number=int(profile['profile'][-2:])
        samples=next(p['samples'] for p in measured if p['profile']==profile['profile'])
        for state in profile['states']:
            folder=root/f'profile-{number:02d}-{state["state"]}';folder.mkdir()
            source=Path(state['sprite_source']);assert sha(source)==state['sprite_sha256']
            rgba=np.array(Image.open(source).convert('RGBA'));alpha=rgba[:,:,3]>=128
            h,w=alpha.shape;x0,y0=state['native_top_left'];yy,xx=np.where(alpha)
            ymin,ymax=int(yy.min()),int(yy.max());rng=np.random.default_rng(number*173+(state['state']=='applied'))
            required=np.zeros((h,w))
            for sample in samples:
                x,y=np.array(sample['pixel'])-[x0,y0]
                if 0<=x<w and 0<=y<h and alpha[y,x] and sample['world']:
                    required[y,x]=max(0,sample['world'][2]+2*SIN)
            # Smooth conservative surface height around existing wood contacts;
            # never lower a source-supported clearance constraint.
            shoulder=maximum_filter(required,size=7)
            shoulder=np.maximum(required,gaussian_filter(shoulder,1.2))
            bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
            scene.name='Croisement02 Refinement'
            obj=bpy.data.objects.new(f'Hidden archer{number:02d} {state["state"]} microleaves',bpy.data.meshes.new('Leaf volume'))
            scene.collection.objects.link(obj)
            asset=f'croisement02-hidden-archer-{number:02d}-{state["state"]}'
            obj['asset_group']=asset;obj['source_node']=f'mission-hidden-archer-{number:02d}'
            mats=[material('Exact native leaf front',source,True),material('Own-source paired leaf back',source,False)]
            for mat in mats:one_sided(mat)
            vertices=[];faces=[];uvs=[];slots=[];known=[];alpha_uvs=[];front_depth=np.full((h,w),np.nan)
            def point(sx,sy,d):return np.array([sx,-sy*SIN,-sy*COS])+ray*d
            def pair(points,front_uv,back_uv,front_slot,back_slot,front_known,alpha_uv=None):
                start=len(vertices)
                normal=np.cross(points[1]-points[0],points[2]-points[0])
                order=[0,1,2,3] if np.dot(normal,ray)>0 else [3,2,1,0]
                for back in [False,True]:
                    selected=list(reversed(order)) if back else order
                    off=len(vertices)
                    for i in selected:
                        vertices.append((points[i]-ray*.02 if back else points[i]).tolist())
                        uvs.append(tuple(back_uv[i] if back else front_uv[i]))
                        alpha_uvs.append(tuple(alpha_uv[i] if alpha_uv else front_uv[i]))
                    faces.extend([(off,off+1,off+2),(off,off+2,off+3)])
                    slots.extend([back_slot if back else front_slot]*2);known.extend([front_known and not back]*2)
            # One source-facing fragment per small region; no repeated deep
            # strips. Native UVs and all alpha holes remain exact.
            anchors=0
            for top in range(0,h,2):
                for left in range(0,w,2):
                    right,bottom=min(w,left+2),min(h,top+2)
                    mask=alpha[top:bottom,left:right]
                    if not mask.any():continue
                    cy=(top+bottom)/2;cx=(left+right)/2
                    t=max(0,min(1,(ymax+.5-cy)/max(1,ymax-ymin+1)))
                    z=.8+(ymax+.5-cy)*.8/COS+16*math.sin(math.pi*t)*math.sin(math.pi*cx/w)
                    z=max(z,float(shoulder[top:bottom,left:right][mask].max()))
                    depth=(z+(y0+cy)*COS)/SIN
                    my,mx=np.where(mask)
                    depth+=float(rng.uniform(-2.5,2.5))
                    needed=(required[top:bottom,left:right][mask]+(y0+top+my+.5)*COS)/SIN
                    depth=max(depth,float(needed.max()))
                    minimum=(-(y0+top+my+.5)*COS+SIN*depth).min()
                    depth+=max(0,.8-minimum)/SIN
                    if top<=ymax<bottom and not required[top:bottom,left:right][mask].any():
                        depth-=(minimum+max(0,.8-minimum)-.8)/SIN;anchors+=1
                    front_depth[top:bottom,left:right][mask]=depth
                    coords=[(left,top),(right,top),(right,bottom),(left,bottom)]
                    points=[point(x0+x,y0+y,depth) for x,y in coords]
                    uv=[(x/w,1-y/h) for x,y in coords];pair(points,uv,uv,0,1,True)
            # Compact rotated leaf surfaces give side depth without stretching
            # a large rectangular source image into a long rear strip.
            count=1800 if state['state']=='initial' else 1100;tile=24;columns=48
            atlas=np.zeros((math.ceil(count/columns)*tile,columns*tile,4),dtype=np.uint8)
            pending=[]
            for n in range(count):
                k=int(rng.integers(len(xx)));cx,cy=float(xx[k])+.5,float(yy[k])+.5
                sx,sy=x0+cx,y0+cy
                d=front_depth[int(cy),int(cx)]-float(rng.uniform(5,w*.65))
                center=point(sx,sy,d)
                normal=rng.normal(size=3);normal/=np.linalg.norm(normal)
                if normal@ray<0:normal=-normal
                axis=np.cross(normal,[0,0,1])
                if np.linalg.norm(axis)<.1:axis=np.cross(normal,[1,0,0])
                axis/=np.linalg.norm(axis);other=np.cross(normal,axis)
                radius=float(rng.uniform(1.2,3.2));axis*=radius;other*=radius*.8
                points=np.array([center-axis-other,center+axis-other,center+axis+other,center-axis+other])
                if points[:,2].min()<.8:points+=ray*((.8-points[:,2].min())/SIN)
                projected=np.column_stack([points[:,0]-x0,-points[:,1]*SIN-points[:,2]*COS-y0])
                l,t=np.floor(projected.min(0)).astype(int);r,b=np.ceil(projected.max(0)).astype(int)
                depths=front_depth[max(0,t):min(h,b+1),max(0,l):min(w,r+1)]
                finite=depths[np.isfinite(depths)]
                if not len(finite):continue
                retreat=max(0,float((points@ray).max()-finite.min()+1))
                if (points-ray*retreat)[:,2].min()<.5:continue
                points-=ray*retreat
                tx,ty=(n%columns)*tile,(n//columns)*tile
                u,v=np.meshgrid((np.arange(tile)+.5)/tile,(np.arange(tile)+.5)/tile)
                world=points[0]+u[:,:,None]*(points[1]-points[0])+v[:,:,None]*(points[3]-points[0])
                gx=np.floor(world[:,:,0]-x0).astype(int);gy=np.floor(-world[:,:,1]*SIN-world[:,:,2]*COS-y0).astype(int)
                donor_x=np.clip((cx+(u-.5)*5).astype(int),0,w-1);donor_y=np.clip((cy+(v-.5)*5).astype(int),0,h-1)
                patch=rgba[donor_y,donor_x].copy()
                valid=(gx>=0)&(gx<w)&(gy>=0)&(gy<h)
                cover=np.zeros_like(valid);cover[valid]=alpha[gy[valid],gx[valid]]
                patch[:,:,3]*=cover
                atlas[ty:ty+tile,tx:tx+tile]=patch
                au=[(tx/atlas.shape[1],1-ty/atlas.shape[0]),((tx+tile)/atlas.shape[1],1-ty/atlas.shape[0]),((tx+tile)/atlas.shape[1],1-(ty+tile)/atlas.shape[0]),(tx/atlas.shape[1],1-(ty+tile)/atlas.shape[0])]
                native_uv=[((p[0]-x0)/w,1-(-p[1]*SIN-p[2]*COS-y0)/h) for p in points]
                pending.append((points,native_uv,au))
            offmap_pairs=0
            if y0==0:
                # Continue the clipped top beyond the map using compact gray
                # leaves. This is inferred geometry, not extra native artwork.
                edge=np.where(alpha[0])[0]
                for n in range(420):
                    donor=int(rng.choice(edge));sy=-float(rng.uniform(.2,24))
                    taper=max(.2,1+sy/30)
                    sx=x0+donor+float(rng.normal(0,4*taper))
                    depth=front_depth[0,donor]-float(rng.uniform(0,18))
                    center=point(sx,sy,depth)
                    normal=rng.normal(size=3);normal/=np.linalg.norm(normal)
                    axis=np.cross(normal,[0,0,1]);axis/=max(1e-6,np.linalg.norm(axis))
                    other=np.cross(normal,axis);radius=float(rng.uniform(.8,2.2))*taper
                    points=np.array([center-axis*radius-other*radius,center+axis*radius-other*radius,center+axis*radius+other*radius,center-axis*radius+other*radius])
                    projected_y=-points[:,1]*SIN-points[:,2]*COS
                    if projected_y.max()>=-.05:continue
                    uv=[(max(0,donor-1)/w,1),(min(w,donor+2)/w,1),(min(w,donor+2)/w,1-3/h),(max(0,donor-1)/w,1-3/h)]
                    pair(points,uv,uv,1,1,False);offmap_pairs+=1
            atlas_path=folder/'own-source-interior.png';Image.fromarray(atlas).save(atlas_path)
            front=material('Native-projected interior front',source,True);one_sided(front)
            nodes=front.node_tree.nodes;links=front.node_tree.links
            uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map='Leaf Alpha UV'
            tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(atlas_path));tex.image.pack();tex.interpolation='Closest';tex.extension='CLIP'
            links.new(uvnode.outputs['UV'],tex.inputs['Vector'])
            shader=next(n for n in nodes if n.type=='BSDF_PRINCIPLED');links.new(tex.outputs['Alpha'],shader.inputs['Alpha'])
            back=material('Inferred own-source interior back',atlas_path,False);one_sided(back)
            mats += [front,back]
            for points,native_uv,au in pending:pair(points,native_uv,au,2,3,True,au)
            # Unseen backs remain explicit neutral geometry pending review.
            for hidden in [mats[1],mats[3]]:
                shader=next(n for n in hidden.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
                for key in ['Base Color','Emission Color']:
                    for link in list(shader.inputs[key].links):hidden.node_tree.links.remove(link)
                    shader.inputs[key].default_value=(.35,.35,.35,1)
            shape=replace_mesh(obj,vertices,faces,uvs,mats,slots,known)
            extra=obj.data.uv_layers.new(name='Leaf Alpha UV')
            for loop in obj.data.loops:extra.data[loop.index].uv=alpha_uvs[loop.vertex_index]
            obj['foliage_physical_opacity']=True;obj['state_endpoint']=state['state'];obj['projection_component']='crown'
            bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True)
            pts=np.array(vertices)
            write_json(folder/'construction.json',dict(status='Private microleaf revision; review pending',asset_id=asset,
                model_sha256=sha(folder/'model.blend'),source=str(source),source_sha256=state['sprite_sha256'],
                source_top_left=[x0,y0],source_opaque_centers=int(alpha.sum()),shape=shape,packet_stride=8,
                geometry_parameters=dict(native_fragment_size=2,fragment_ray_jitter=2.5,interior_attempts=count),
                interior_leaf_pairs=len(pending),offmap_gray_leaf_pairs=offmap_pairs,ground_fringe_anchor_fragments=anchors,
                world_min=pts.min(0).tolist(),world_max=pts.max(0).tolist(),
                limitations=['Hidden leaves reuse only this endpoint native image; not observed rear artwork.',
                'Ground fringe anchors are inferred geometry; complete support/contact checks still required.',
                'No existing static geometry, material, or state contract changed.']))
    from restart14_hidden_archer_review import main as review
    review(root)


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
