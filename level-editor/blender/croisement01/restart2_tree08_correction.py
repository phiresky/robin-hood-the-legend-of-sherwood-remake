"""Private Tree08 correction recipe. Depth hypotheses preserve source centers."""
import math
import numpy as np

# Primary bough anchors are native source coordinates; depth is inferred.
# Values interpolate smoothly from the trunk, rather than alternating per twig.
DEPTH_FAMILIES = (
    ('left-upper', (422, 102), -210.0, 95.0),
    ('central-upper', (510, 92), 220.0, 65.0),
    ('right-upper', (594, 80), -180.0, 65.0),
    ('right-middle', (706, 214), 215.0, 90.0),
    ('right-low', (682, 285), -170.0, 80.0),
    ('left-low', (414, 183), 150.0, 75.0),
)
CROSSING_BOX = (685, 185, 777, 315)


def smoothstep(t):
    t = max(0.0, min(1.0, t))
    return t*t*(3-2*t)


def branch_depth(x, y):
    """Smooth primary-family depth field, zero at trunk/flare attachment."""
    trunk_x = np.interp(y, [200, 221, 283, 333, 369, 405], [552, 556, 564, 560, 557, 567])
    # No sudden depth jump at a branch base or at the lower root interface.
    growth = smoothstep(math.hypot(x-trunk_x, max(0, 218-y)) / 95)
    weights = [math.exp(-((x-a[0])**2+(y-a[1])**2)/(.5*sigma*sigma)) for _, a, _, sigma in DEPTH_FAMILIES]
    return growth*sum(w*f[2] for w,f in zip(weights,DEPTH_FAMILIES))/max(sum(weights),1e-20)


def native_center(x, y):
    s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
    # Smooth transition from upright stem to descending surface root.
    t = smoothstep((y-350)/40)
    vertical_z = (370-y)/c
    root_z = -(y-370)*.25
    z = vertical_z*(1-t)+root_z*t
    ray = np.array([0, -c, s])
    return np.array([x, -(y+z*c)/s, z]) + ray*branch_depth(x,y)


def smooth_radii(path):
    """Suppress skeleton-node width spikes; later coverage checks stay mandatory."""
    raw=np.array([p[2] for p in path],dtype=float)*1.14
    if len(raw)<2:return np.maximum(raw,.65)
    # Robust local median, then symmetric weighted averaging, no knot spheres.
    med=np.array([np.median(raw[max(0,i-6):min(len(raw),i+7)]) for i in range(len(raw))])
    radii=np.array([np.mean(med[max(0,i-4):min(len(raw),i+5)]) for i in range(len(raw))])
    # Limit radius slope in both directions while leaving source centers exact.
    for order in (range(1,len(radii)),range(len(radii)-2,-1,-1)):
        for i in order:
            j=i-1 if order.step>0 else i+1
            step=math.dist(path[i][:2],path[j][:2]);radii[i]=min(radii[i],radii[j]+.3*step)
    return np.maximum(radii,.65)


def build_sections(trace, selected, prior_misses):
    """Return closed tubes plus audit records; do not write or mutate assets."""
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
    ray=np.array([0,-c,s]);down=np.array([0,-s,-c]);right=np.array([1,0,0])
    sections=[];miss_assignment=[]
    # Distinguish all prior misses. Local geometric support only; no alpha edits.
    samples=[(i,j,p) for i in selected for j,p in enumerate(trace[i])]
    xy=np.array([p[:2] for _,_,p in samples])
    needed={}
    for miss in prior_misses:
        k=int(np.argmin(np.sum((xy-(np.array(miss)+.5))**2,axis=1)));i,j,p=samples[k]
        gap=float(np.linalg.norm(xy[k]-(np.array(miss)+.5)))
        supported=gap<=max(3,p[2]*1.4)
        miss_assignment.append(dict(native=miss,trace_id=i,sample=j,distance=gap,action='local swept-radius support with post-remesh reaudit' if supported else 'unresolved remote source; do not bridge or delete'))
        if supported:needed[i,j]=max(needed.get((i,j),0),gap+.8)
    for trace_id in selected:
        path=trace[trace_id]
        if len(path)<2:continue
        radii=smooth_radii(path)
        # Spread measured support over a smooth longitudinal neighborhood.
        for j in range(len(path)):
            required=needed.get((trace_id,j),0)
            if required:
                for k in range(max(0,j-8),min(len(path),j+9)):
                    w=math.exp(-((k-j)/4)**2)
                    radii[k]=max(radii[k],required*w)
        vertices=[];faces=[];n=16
        for j,(x,y,_) in enumerate(path):
            before=path[max(0,j-1)];after=path[min(len(path)-1,j+1)]
            dx,dy=after[0]-before[0],after[1]-before[1];length=max(.001,math.hypot(dx,dy));normal=right*(-dy/length)+down*(dx/length)
            center=native_center(x,y)
            for k in range(n):
                a=math.tau*k/n;vertices.append(center+normal*(math.cos(a)*radii[j])+ray*(math.sin(a)*radii[j]))
        for j in range(len(path)-1):
            for k in range(n):a=j*n+k;b=j*n+(k+1)%n;faces.append((a,b,b+n,a+n))
        faces.extend([tuple(reversed(range(n))),tuple((len(path)-1)*n+k for k in range(n))])
        x0,y0,x1,y1=CROSSING_BOX
        held=any(x0<=p[0]<=x1 and y0<=p[1]<=y1 for p in path)
        sections.append(dict(trace_id=trace_id,vertices=vertices,faces=faces,held_crossing=held))
    # Continuous inferred basal core below the observed flare. Its rounded tip
    # is for burial; it does not claim contact with a yet-unloaded soil receiver.
    rings=[(557,350,19),(557,360,21),(558,371,23),(559,381,22),(561,390,18),(565,400,12),(568,409,5),(569,414,.4)]
    vertices=[];faces=[];n=24
    for x,y,r in rings:
        for k in range(n):
            a=math.tau*k/n;vertices.append(native_center(x,y)+right*(math.cos(a)*r)+ray*(math.sin(a)*r))
    for j in range(len(rings)-1):
        for k in range(n):a=j*n+k;b=j*n+(k+1)%n;faces.append((a,b,b+n,a+n))
    faces.extend([tuple(reversed(range(n))),tuple((len(rings)-1)*n+k for k in range(n))])
    sections.append(dict(trace_id='inferred-basal-continuation',vertices=vertices,faces=faces,held_crossing=False))
    return sections,miss_assignment


def consolidate(bpy, collection, sections):
    """Union only the connected main family; retain disputed crossing parts."""
    objects=[];core_vertices=[];core_faces=[]
    for section in sections:
        if not section['held_crossing']:
            offset=len(core_vertices);core_vertices.extend(section['vertices']);core_faces.extend(tuple(offset+i for i in f) for f in section['faces']);continue
        mesh=bpy.data.meshes.new('Unresolved crossing section');mesh.from_pydata(section['vertices'],[],section['faces']);mesh.update();obj=bpy.data.objects.new(f"Held crossing {section['trace_id']}",mesh);collection.objects.link(obj);objects.append(obj)
    mesh=bpy.data.meshes.new('Main coherent wood volume');mesh.from_pydata(core_vertices,[],core_faces);mesh.update();obj=bpy.data.objects.new('Main wood with basal continuation',mesh);collection.objects.link(obj);bpy.context.view_layer.objects.active=obj;obj.select_set(True)
    modifier=obj.modifiers.new('Consolidate confirmed primary family','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.8;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
    # Small uniform surface smoothing, never a substitute for source reaudit.
    modifier=obj.modifiers.new('Soften junction shoulders','SMOOTH');modifier.factor=.35;modifier.iterations=2;bpy.ops.object.modifier_apply(modifier=modifier.name)
    for face in obj.data.polygons:face.use_smooth=True
    objects.insert(0,obj)
    # Combine containers without welding held crossing geometry.
    for other in bpy.context.selected_objects:other.select_set(False)
    for other in objects:other.select_set(True)
    bpy.context.view_layer.objects.active=obj;bpy.ops.object.join()
    return obj
