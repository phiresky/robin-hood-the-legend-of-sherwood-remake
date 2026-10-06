"""Fit and inspect a bounded intact wagon without assigning its horses or ground to wood."""
import json, math, sys, shutil
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'), str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
VARIANT = sys.argv[sys.argv.index('--variant') + 1] if '--variant' in sys.argv else 'v1'
DEST = OUT / f'restart3-south-cart/initial-physical-{VARIANT}'
AUDIT = sys.argv[sys.argv.index('--audit-version')+1] if '--audit-version' in sys.argv else 'current-support-v1'
SURVEY = OUT / 'restart3-south-cart/initial-source-survey-v1'
ROOF = [(154,49),(158,36),(167,23),(177,17),(187,18),(278,47),(274,55),(259,70),(243,68),(198,56),(167,53)]
WALLS = [(158,52),(168,55),(253,75),(274,57),(277,85),(267,95),(250,97),(170,78),(161,83)]
PLATFORM = [(126,76),(138,64),(145,64),(155,53),(162,59),(165,74),(146,85),(137,85)]

def project(v): return [(p[0],-p[1]*SIN-p[2]*COS) for p in v]
def roof_geometry(params):
    cx,cy,length,angle,width,rise = params
    u=np.array([math.cos(angle),math.sin(angle),0.]);v=np.array([-u[1],u[0],0.]);eave=60.
    origin=np.array([1049+cx,-(765+cy+eave*COS)/SIN,eave]);verts=[];n=24
    for a in [0.,length]:
        for thickness in [0.,-2.]:
            for i in range(n+1):
                t=math.pi*i/n;verts.append((origin+u*a+v*(width*math.cos(t))+np.array([0,0,rise*math.sin(t)+thickness])).tolist())
    s=n+1;faces=[]
    for i in range(n):
        for a,b in [(0,2*s),(s,3*s),(0,s),(2*s,3*s)]: faces.append([a+i,a+i+1,b+i+1,b+i])
    for i in [0,n]:faces.append([i,s+i,3*s+i,2*s+i])
    return verts,faces,origin,u,v

def prepare():
    from scipy.optimize import differential_evolution
    assert shutil.disk_usage(OUT).free>25*1024**3
    DEST.mkdir(exist_ok=False)
    source=OUT/'state-target-evidence/profiles/chariot02-07/action-160-direction-0-frame-000.png'
    assert sha(source)=='471253207d320bf2b61b2d26a54f3a82549f86e4f44b9eaa4717da6bb19564f1'
    rgba=np.array(Image.open(source).convert('RGBA'));domain=Image.new('L',(288,178));ImageDraw.Draw(domain).polygon(ROOF,fill=255);expected=np.asarray(domain)>0
    def mask(params):
        vs,fs,*_=roof_geometry(params);points=[(x-1049,y-765) for x,y in project(vs)];im=Image.new('L',(288,178));d=ImageDraw.Draw(im)
        for f in fs:d.polygon([points[i] for i in f],fill=255)
        return np.asarray(im)>0
    def objective(p):
        m=mask(p);return 2*np.count_nonzero(expected&~m)+np.count_nonzero(m&~expected)
    result=differential_evolution(objective,[(165,185),(32,52),(95,125),(-.62,-.38),(19,35),(12,33)],seed=512,maxiter=90,popsize=12,polish=False)
    roles={};full=np.zeros((178,288),bool)
    for name,polygon in [('roof',ROOF),('fore_platform',PLATFORM)]+([('cabin_walls',WALLS)] if VARIANT in ('v3','v4','v5','v6','v7','v8') else []):
        im=Image.new('L',(288,178));ImageDraw.Draw(im).polygon(polygon,fill=255);scope=(np.asarray(im)>0)&(rgba[:,:,3]>127)&~full;roles[name]={'polygon':polygon,'known_pixels':int(scope.sum())};full|=scope
        a=rgba.copy();a[:,:,3]=np.where(scope,a[:,:,3],0);Image.fromarray(a).save(DEST/f'{name}-source.png')
    scoped=rgba.copy();scoped[:,:,3]=np.where(full,scoped[:,:,3],0);Image.fromarray(scoped).save(DEST/'source.png')
    overlay=Image.new('RGBA',(288,178),(65,70,75,255));overlay.alpha_composite(Image.fromarray(rgba));d=ImageDraw.Draw(overlay);d.line(ROOF+[ROOF[0]],fill=(255,200,20,255),width=1);d.line(PLATFORM+[PLATFORM[0]],fill=(20,220,255,255),width=1);overlay.resize((1152,712),Image.Resampling.NEAREST).save(DEST/'source-roles.png')
    m=mask(result.x);report={'status':'Private initial hypothesis; surveyed wagon roles are native, hidden running gear inferred, actors and ground separate','source':str(source),'source_sha256':sha(source),'source_box':[1049,765,1337,943],'source_roles_disjoint':True,'parameters':result.x.tolist(),'roles':roles,'roof_fit':{'missing_polygon_pixels':int((expected&~m).sum()),'extra_polygon_pixels':int((m&~expected).sum())},'native_pixel_count':int((rgba[:,:,3]>127).sum()),'known_wagon_pixels':int(full.sum()),'unassigned_pixels':int(((rgba[:,:,3]>127)&~full).sum()),'limitations':['Unassigned includes horses, harness, uncertain dark underbody and ground/shadow fragments; not all omitted pixels are wagon defects.','Later moving frames are inspection references only, never transplanted into frame0.','Four wheels and hidden bed are a support hypothesis; their initial centers are not recoverable from the foliage-clipped native source.','Serialized target31 starts action160 frame0; targets32/33 start transparent. No new physical motion claim.']}
    write_json(DEST/'fit.json',report);print(json.dumps(report,indent=2))

def build():
    import bpy,bmesh
    from mathutils import Vector
    from mathutils.bvhtree import BVHTree
    from scenery_geometry import Mesh
    from tree_geometry import RAY
    from log_trap_state_candidate import material,point
    from render_slots import acquire,release
    fit=json.loads((DEST/'fit.json').read_text());assert shutil.disk_usage(OUT).free>25*1024**3
    if (DEST/'worker.blend').exists():raise FileExistsError(DEST/'worker.blend')
    acquire()
    try:
        bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.render.image_settings.color_mode='RGBA';scene.render.resolution_x=scene.render.resolution_y=320;scene.render.resolution_percentage=100
        scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.22,.22,.22,1)
        gray=bpy.data.materials.new('Unobserved intact wagon material');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.24,.24,.24,1)
        roofmat=material(DEST/'roof-source.png');platformmat=material(DEST/'fore_platform-source.png');wallmat=material(DEST/'cabin_walls-source.png') if VARIANT in ('v3','v4','v5','v6','v7','v8') else None
        vs,fs,origin,u,v=roof_geometry(fit['parameters']);length=fit['parameters'][2];width=fit['parameters'][4]-2;pieces=[('Straw canopy shell',vs,fs,roofmat)]
        base=origin.copy();base[2]=0
        def world(a,b,z):return (base+u*a+v*b+np.array([0,0,z])).tolist()
        def box(name,aa,bb,zz,paint=None):
            vv=[world(a,b,z) for z in zz for b in bb for a in aa];ff=[[0,1,3,2],[4,6,7,5],[0,4,5,1],[2,3,7,6],[0,2,6,4],[1,5,7,3]];pieces.append((name,vv,ff,paint))
        box('Continuous bed',(10 if VARIANT=='v8' else (-1 if VARIANT in ('v4','v5','v6','v7','v8') else -17),length-2),(-width+2,width-2),(25,29))
        if VARIANT == 'v1':
            box('Forward loading platform',(-32,-1),(-width+2,width-2),(27,30),platformmat)
        elif VARIANT in ('v5','v6','v7','v8'):
            corners=[(122,77),(155,49),(170,75),(137,92)] if VARIANT=='v8' else [(124,76),(155,51),(169,75),(137,90)];upper=[]
            for x,y in corners:
                z=(-.06674613*x-.30154946*y+41.32777116) if VARIANT in ('v6','v7','v8') else 30+.6*(x-145)-.2*(y-70);upper.append(np.array([1049+x,-(765+y+z*COS)/SIN,z]))
            normal=np.cross(upper[1]-upper[0],upper[3]-upper[0]);normal/=np.linalg.norm(normal)
            if normal[2]<0:normal=-normal
            vv=[p.tolist() for p in upper]+[(p-normal*2).tolist() for p in upper];ff=[[0,1,2,3],[7,6,5,4],[0,4,5,1],[1,5,6,2],[2,6,7,3],[3,7,4,0]]
            pieces.append(('Inclined front timber panel',vv,ff,platformmat))
            for i,k in enumerate([1,2]):
                m=Mesh();m.tube(Vector(upper[k]-normal),Vector(world(12 if VARIANT=='v8' else 0,(-1 if i else 1)*(width-4),27)),1.8,n=8);pieces.append((f'Panel frame brace {i}',m.vertices,m.faces,None))
        else:
            box('Lower forward loading platform',(-33,-7),(-width+2,width-2),(10,13),platformmat)
            for side in [-1,1]:
                m=Mesh();m.tube(Vector(world(-8,side*(width-4),12)),Vector(world(1,side*(width-4),27)),1.8,n=8);pieces.append((f'Platform support {side}',m.vertices,m.faces,None))
        for side in [-1,1]:
            b=side*(width-2);box(f'Side boards {side}',(11 if VARIANT in ('v7','v8') else -1,length-2),(b-1,b+1),(29,60+fit['parameters'][5]*math.sqrt(1-(b/fit['parameters'][4])**2)-1 if VARIANT in ('v3','v4','v5','v6','v7','v8') else 44),wallmat)
            for a in [12 if VARIANT in ('v7','v8') else 1,length-3]:box(f'Canopy post {a:.1f} {side}',(a-1.5,a+1.5),(b-1.5,b+1.5),(27,60+fit['parameters'][5]*math.sqrt(1-(b/fit['parameters'][4])**2)-1.0))
        for a in [11 if VARIANT in ('v7','v8') else 0,length-3]:box(f'End board {a:.1f}',(a-1,a+1),(-width+2,width-2),(29,60 if VARIANT in ('v3','v4','v5','v6','v7','v8') else 43),wallmat)
        # Axle centers are inferred from a stable four-wheel footprint, not copied from the wreck.
        radius=18.;axis=Vector(v)
        for axle,a in enumerate([20 if VARIANT in ('v4','v5','v6','v7','v8') else 4,length-15]):
            for side in [-1,1]:
                b=side*(width-5);box(f'Axle bearing {axle} {side}',(a-3,a+3),(b-3,b+3),(16,27))
            m=Mesh();m.tube(Vector(world(a,-width-3,radius)),Vector(world(a,width+3,radius)),2,n=12);pieces.append((f'Axle {axle}',m.vertices,m.faces,None))
            for side in [-1,1]:
                center=Vector(world(a,side*(width+2),radius));vv=[];n=32
                for depth in [-1.6,1.6]:
                    for rad in [radius,radius-3]:
                        for k in range(n):
                            t=math.tau*k/n;vv.append(tuple(center+axis*depth+Vector(u)*(rad*math.cos(t))+Vector((0,0,rad*math.sin(t)))))
                ff=[]
                for k in range(n):
                    j=(k+1)%n
                    for x,y in [(0,n),(2*n,3*n),(0,2*n),(n,3*n)]:ff.append([x+k,x+j,y+j,y+k])
                pieces.append((f'Wheel rim {axle} {side}',vv,ff,None));m=Mesh();m.tube(center-axis*3,center+axis*3,3.6,n=12)
                for k in range(8):
                    t=math.tau*k/8;d=Vector(u)*math.cos(t)+Vector((0,0,math.sin(t)));m.tube(center+d*2,center+d*16,1.2,n=6)
                pieces.append((f'Hub and spokes {axle} {side}',m.vertices,m.faces,None))
        if VARIANT in ('v3','v4','v5','v6','v7','v8'):
            for label,a in [('Front',11 if VARIANT in ('v7','v8') else 0),('Rear',length-3)]:
                vv=[];n=24
                for aa in [a-1,a+1]:
                    vv.append(world(aa,0,58))
                    for k in range(n+1):
                        t=math.pi*k/n;vv.append(world(aa,width*math.cos(t),58+fit['parameters'][5]*math.sin(t)))
                st=n+2;ff=[]
                for k in range(1,n+1):ff.extend([[0,k,k+1],[st,st+k+1,st+k],[k,k+1,st+k+1,st+k]])
                ff.extend([[0,st,st+1,1],[0,n+1,st+n+1,st]])
                pieces.append((label+' arched cabin closure',vv,ff,wallmat))
        objects=[];audit=[]
        for name,vv,ff,paint in pieces:
            data=bpy.data.meshes.new(name);data.from_pydata(vv,[],ff);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bmesh.ops.triangulate(bm,faces=list(bm.faces));volume=bm.calc_volume();closed=all(e.is_manifold for e in bm.edges)
            if paint:bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=4,use_grid_fill=True)
            bm.to_mesh(data);bm.free();data.update();ob=bpy.data.objects.new(name,data);scene.collection.objects.link(ob);objects.append(ob);data.materials.append(gray)
            if paint:data.materials.append(paint)
            uv=data.uv_layers.new(name='Native target projection')
            for face in data.polygons:
                face.material_index=1 if paint and face.normal.dot(RAY)>.04 else 0
                for loop in face.loop_indices:
                    p=data.vertices[data.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-1049)/288,1-(-p.y*SIN-p.z*COS-765)/178)
            ob['geometry_status']='Private initial wagon, source roles bounded';ob['inferred_geometry']=paint is None
            audit.append({'name':name,'closed_manifold':closed,'volume':volume,'minimum_z':min(p[2] for p in vv)})
        lamp=bpy.data.lights.new('Sun','SUN');lamp.energy=2;ob=bpy.data.objects.new('Sun',lamp);scene.collection.objects.link(ob);ob.rotation_euler=(.6,-.5,-.4)
        cd=bpy.data.cameras.new('Native first');cd.type='ORTHO';cd.clip_end=10000;cam=bpy.data.objects.new('Native first',cd);scene.collection.objects.link(cam);scene.camera=cam
        assert shutil.disk_usage(OUT).free>25*1024**3
        bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'worker.blend'),compress=True)
        bounds=[p.co for o in objects for p in o.data.vertices];center=Vector(tuple((min(p[i] for p in bounds)+max(p[i] for p in bounds))/2 for i in range(3)));scale=max((max(p[i] for p in bounds)-min(p[i] for p in bounds))for i in range(3))*1.35
        for mode in ['actual','solid']:
            sheet=Image.new('RGBA',(1280,640))
            scene.view_layers[0].material_override=gray if mode=='solid' else None
            for i in range(8):
                angle=math.tau*i/8;direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();cd.ortho_scale=scale;scene.render.filepath=str(DEST/f'{mode}-{i}.png');bpy.ops.render.render(write_still=True);sheet.paste(Image.open(scene.render.filepath),(320*(i%4),320*(i//4)))
            sheet.save(DEST/f'{mode}-eight.png')
        # Exact native art canvas; no fitting a camera to arbitrary azimuth zero.
        cam.location=point(1193,854,0)+RAY*3000;cam.rotation_euler=(point(1193,854,0)-cam.location).to_track_quat('-Z','Y').to_euler();cd.sensor_fit='HORIZONTAL';cd.ortho_scale=288;scene.render.resolution_x=1152;scene.render.resolution_y=712;scene.view_layers[0].material_override=None;scene.render.filepath=str(DEST/'native-actual.png');bpy.ops.render.render(write_still=True)
        write_json(DEST/'manifest.json',{'status':'Private candidate; requires source/actual/support self-review before root','model_sha256':sha(DEST/'worker.blend'),'fit_sha256':sha(DEST/'fit.json'),'components':audit,'original_camera_first':True,'native_direction':list(RAY),'limitations':fit['limitations']+['Assigned source RGB is limited to explicitly surveyed roof/platform and, in v3, cabin walls; wheels and obscured geometry remain inferred neutral material.','Support analytic at z0 pending evaluated approved receiver audit.']})
    finally:release()
def support():
    import bpy
    from mathutils import Vector
    from render_slots import acquire, release
    import restart3_north_cart_support as support_audit
    from tree_geometry import RAY
    support_audit.MODELS = [('ground', OUT/'restart2-state/trap-ground-continuation-model-v1/model.blend', 'fe8da24ebb5f696f8df6453baf2a82454737bf4157f1a65ecdf78b2380d8e7ec'), support_audit.MODELS[1]]
    acquire()
    try:
        support_audit.audit(DEST, DEST/AUDIT)
        scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH']
        from mathutils.bvhtree import BVHTree
        trees={o.name:BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[p.vertices[:] for p in o.data.polygons]) for o in objects}
        graph={n:[] for n in trees}
        for i,a in enumerate(trees):
            for b in list(trees)[i+1:]:
                if trees[a].overlap(trees[b]):graph[a].append(b);graph[b].append(a)
        connected={'Continuous bed'}
        while True:
            more=connected|set().union(*(set(graph[n]) for n in connected))
            if more==connected:break
            connected=more
        write_json(DEST/AUDIT/'connectivity.json',{'model_sha256':sha(DEST/'worker.blend'),'surface_graph':graph,'connected_to_bed':sorted(connected),'disconnected':sorted(set(trees)-connected),'method':'Exact mesh triangle overlap; containment without boundary intersection is not counted.'})
        total_volume=0.;moment=np.zeros(3)
        for o in objects:
            data=o.data;data.calc_loop_triangles();ref=np.array(o.matrix_world@data.vertices[0].co);vol=0.;mom=np.zeros(3)
            for t in data.loop_triangles:
                a,b,c=[np.array(o.matrix_world@data.vertices[i].co)-ref for i in t.vertices];signed=float(np.dot(a,np.cross(b,c))/6);vol+=signed;mom+=signed*(ref+(a+b+c)/4)
            total_volume+=abs(vol);moment+=mom*(1 if vol>0 else -1)
        com=moment/total_volume
        supports=json.loads((DEST/AUDIT/'report.json').read_text());contacts=sorted(set((float(p[0]),float(p[1]))for row in supports['objects'] if row['object'].startswith('Wheel rim')for p in row['contacts']))
        def cross(a,b,c):return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
        lower=[];upper=[]
        for p in contacts:
            while len(lower)>1 and cross(lower[-2],lower[-1],p)<=0:lower.pop()
            lower.append(p)
        for p in reversed(contacts):
            while len(upper)>1 and cross(upper[-2],upper[-1],p)<=0:upper.pop()
            upper.append(p)
        hull=lower[:-1]+upper[:-1]
        if len(hull)<3:raise ValueError('Insufficient wheel support polygon')
        margin=min(cross(a,b,com)/math.hypot(b[0]-a[0],b[1]-a[1]) for a,b in zip(hull,hull[1:]+hull[:1]))
        write_json(DEST/AUDIT/'static-support.json',{'model_sha256':sha(DEST/'worker.blend'),'uniform_density_com':com.tolist(),'support_hull':hull,'inside_margin':margin,'pass':bool(margin>0),'scope':'Uniform-density closed-component support plausibility; component overlap is counted separately, not a dynamic load simulation.'})
        allverts=[];allfaces=[];owners=[]
        for o in objects:
            start=len(allverts);allverts.extend([o.matrix_world@v.co for v in o.data.vertices]);allfaces.extend([tuple(start+i for i in p.vertices)for p in o.data.polygons]);owners.extend([o.name]*len(o.data.polygons))
        fulltree=BVHTree.FromPolygons(allverts,allfaces);coverage={}
        from log_trap_state_candidate import point
        for role,expected_owner in [('roof','Straw canopy shell'),('fore_platform','Forward loading platform' if VARIANT=='v1' else ('Inclined front timber panel' if VARIANT in ('v5','v6','v7','v8') else 'Lower forward loading platform'))]+([('cabin_walls','Side boards -1')] if VARIANT in ('v3','v4','v5','v6','v7','v8') else []):
            rgba=np.array(Image.open(DEST/f'{role}-source.png'));yy,xx=np.where(rgba[:,:,3]>127);counts={};miss=0;residual=[]
            for y,x in zip(yy,xx):
                hit=fulltree.ray_cast(point(1049+x+.5,765+y+.5,0)+RAY*3000,-RAY,6000)
                if hit[0] is None:miss+=1;residual.append([int(x),int(y),None])
                else:
                    owner=owners[hit[2]];counts[owner]=counts.get(owner,0)+1
                    if owner!=expected_owner:residual.append([int(x),int(y),owner])
            coverage[role]={'known_pixels':len(xx),'first_hit_counts':counts,'no_hit':miss,'expected_owner':expected_owner,'matching_owner_pixels':counts.get(expected_owner,0),'other_or_missing_first_hit_samples':residual}
        write_json(DEST/AUDIT/'source-first-hits.json',{'model_sha256':sha(DEST/'worker.blend'),'roles':coverage,'scope':'Pixel-center first-hit ownership; unknown source outside surveyed domains is not assigned to wagon.'})
        points=[o.matrix_world@v.co for o in objects for v in o.data.vertices]
        lo=Vector(tuple(min(p[i] for p in points)for i in range(3)));hi=Vector(tuple(max(p[i]for p in points)for i in range(3)));center=(lo+hi)/2
        # A neutral contact patch reproduces the verified flat receiver's geometry; it is context only.
        data=bpy.data.meshes.new('Verified flat ground contact context');data.from_pydata([(lo.x-15,lo.y-15,0),(hi.x+15,lo.y-15,0),(hi.x+15,hi.y+15,0),(lo.x-15,hi.y+15,0)],[],[[0,1,2,3]]);ob=bpy.data.objects.new(data.name,data);scene.collection.objects.link(ob)
        scene.render.resolution_x=scene.render.resolution_y=512;scene.camera.data.ortho_scale=max(hi.x-lo.x,hi.y-lo.y)*1.25
        sheet=Image.new('RGBA',(1536,512))
        for i,d in enumerate([RAY,Vector((.9,-.3,.2)).normalized(),Vector((-.4,.9,.2)).normalized()]):
            cam=scene.camera;cam.location=center+d*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(DEST/AUDIT/f'contact-{i}.png');bpy.ops.render.render(write_still=True);sheet.paste(Image.open(scene.render.filepath),(i*512,0))
        sheet.save(DEST/AUDIT/'contact-sheet.png')
    finally:release()

if __name__=='__main__':
    if '--audit' in sys.argv:support()
    elif '--build'  in sys.argv:build()
    else:prepare()
