"""Rooted source-arc depth hypotheses with bounded derivatives, no spatial blend."""
import heapq,math
import numpy as np
from restart2_tree08_correction import smoothstep


def rooted_arcs(traces,selected,root):
    graph={};lengths={}
    for i in selected:
        p=traces[i]
        if len(p)<2:continue
        a,b=tuple(p[0][:2]),tuple(p[-1][:2]);length=sum(math.dist(x[:2],y[:2]) for x,y in zip(p,p[1:]));lengths[i]=length
        graph.setdefault(a,[]).append((b,i,length));graph.setdefault(b,[]).append((a,i,length))
    root=min(graph,key=lambda p:math.dist(p,root));dist={root:0};incoming={};q=[(0,root)]
    while q:
        d,a=heapq.heappop(q)
        if d!=dist[a]:continue
        for b,i,length in graph[a]:
            if d+length<dist.get(b,float('inf')):
                dist[b]=d+length;incoming[b]=(a,i);heapq.heappush(q,(d+length,b))
    # A non-tree source arc can be a projected crossing. It is still retained,
    # but attaches only at its near end, not welded automatically at both ends.
    tree_ids={i for _,i in incoming.values()};depth={root:0.0};slope={root:0.0};plans={};pending=set(lengths)
    while pending:
        progress=False
        for i in sorted(pending,key=lambda j:min(dist.get(tuple(traces[j][0][:2]),1e30),dist.get(tuple(traces[j][-1][:2]),1e30))):
            path=traces[i];a,b=tuple(path[0][:2]),tuple(path[-1][:2])
            if dist.get(a,1e30)>dist.get(b,1e30):path=path[::-1];a,b=b,a
            if a not in depth:continue
            progress=True;pending.remove(i)
            # Main stem and descending roots keep their independent soil-slope
            # hypothesis. Boughs inherit at origin, then bend gradually.
            mx=sum(p[0] for p in path)/len(path);my=sum(p[1] for p in path)/len(path)
            stem=my>220 and abs(mx-560)<35
            if stem:target=0.0
            else:
                dx=path[-1][0]-path[0][0];dy=path[-1][1]-path[0][1]
                # Coherent signs by primary source limb; tiny child arcs inherit
                # rather than receiving alternating random depth instructions.
                if lengths[i]<18:target=slope[a]
                elif mx<485:target=-.55
                elif mx<550:target=.55
                elif mx<630:target=-.4
                elif my<255:target=.55
                else:target=-.45
            points=[];d=depth[a];v=slope[a];arc=0.0
            for j,p in enumerate(path):
                if j:
                    step=math.dist(path[j-1][:2],p[:2]);arc+=step
                    nv=v+max(-.018*step,min(.018*step,target-v));nv=max(-.6,min(.6,nv));d+=(v+nv)*.5*step;v=nv
                points.append([*p,float(d),float(v),float(arc)])
            plans[i]=dict(trace_id=i,source_points=points,origin=list(a),tip=list(b),tree_arc=i in tree_ids,inherited_depth=depth[a],inherited_slope=slope[a],target_slope=target)
            if i in tree_ids and incoming.get(b)==(a,i):depth[b]=d;slope[b]=v
        if not progress:break
    return plans,sorted(pending),dict(root=list(root),connected_nodes=len(dist),tree_arc_count=len(tree_ids),max_abs_slope=max((abs(p[4]) for r in plans.values() for p in r['source_points']),default=0),max_depth_curvature=.018)


def regularize_radii(path):
    """Smooth source radius along arc length, without using depth as width."""
    raw=np.array([p[2] for p in path],float)*1.14
    # Radius estimate within a skeleton junction can reflect the union of two
    # branches. A lower local quantile avoids turning it into a spherical knot.
    result=np.array([np.quantile(raw[max(0,j-12):min(len(raw),j+13)],.4) for j in range(len(raw))])
    for _ in range(3):result=np.array([np.mean(result[max(0,j-6):min(len(raw),j+7)]) for j in range(len(raw))])
    for order in (range(1,len(result)),range(len(result)-2,-1,-1)):
        for j in order:
            k=j-1 if order.step>0 else j+1
            result[j]=min(result[j],result[k]+.16*math.dist(path[j][:2],path[k][:2]))
    return np.maximum(result,.65)


def build_hierarchy_sections(traces,selected,root,misses,support_core=None,continuous_nodes=False,continuous_trunk=False,transport_frames=False):
    plans,pending,info=rooted_arcs(traces,selected,root)
    assert not pending,('Unassigned source arcs',pending)
    if continuous_trunk:
        # These source arcs are successive samples of the same uninterrupted
        # stem, not separate cylinders or independent branch origins.
        trunk_ids=[165,179,187,188,190,207,223,233]
        joined=[];cursor=(556,221)
        for i in trunk_ids:
            pts=plans[i]['source_points']
            if tuple(pts[-1][:2])==cursor:pts=list(reversed(pts))
            assert tuple(pts[0][:2])==cursor,(i,cursor,pts[0])
            joined.extend(pts if not joined else pts[1:]);cursor=tuple(pts[-1][:2])
        arc=0
        for j,p in enumerate(joined):
            if j:arc+=math.dist(joined[j-1][:2],p[:2])
            p[5]=arc
        for i in trunk_ids:del plans[i]
        plans[10000]=dict(trace_id=10000,source_points=joined,origin=list(joined[0][:2]),tip=list(joined[-1][:2]),tree_arc=True,inherited_depth=joined[0][3],inherited_slope=joined[0][4],target_slope=0,continuous_source_arcs=trunk_ids)
        info['continuous_trunk_source_arcs']=trunk_ids
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=np.array([0,-c,s]);down=np.array([0,-s,-c]);right=np.array([1,0,0])
    def point(x,y,depth):
        t=smoothstep((y-350)/40);z=(370-y)/c*(1-t)-(y-370)*.25*t
        return np.array([x,-(y+z*c)/s,z])+ray*depth
    samples=[(i,j,p) for i,a in plans.items() for j,p in enumerate(a['source_points'])];xy=np.array([p[:2] for _,_,p in samples]);needed={};obligations=[]
    for miss in (support_core if support_core is not None else misses):
        k=int(np.argmin(np.sum((xy-(np.array(miss)+.5))**2,axis=1)));i,j,p=samples[k];distance=float(np.linalg.norm(xy[k]-(np.array(miss)+.5)));supported=distance<=max(3,p[2]*1.4)
        if tuple(miss) in {tuple(q) for q in misses}:obligations.append(dict(native=miss,trace_id=i,sample=j,distance=distance,action='smooth local source support and reaudit' if supported else 'retain unresolved; no automatic joining'))
        if supported:needed[i,j]=max(needed.get((i,j),0),distance+.8)
    sections=[];incident={};endpoint_targets={}
    if continuous_nodes:
        for i,arc in plans.items():
            if not arc['tree_arc']:continue
            ps=arc['source_points'];rr=regularize_radii(ps)
            for j in [0,len(ps)-1]:incident.setdefault(tuple(ps[j][:2]),[]).append((i,max(float(rr[j]),ps[j][2]*1.14)))
        # Degree-two nodes are samples on one limb, not anatomical forks.
        # Give both tube ends the same radius and transition gradually.
        endpoint_targets={p:max(v for _,v in links) for p,links in incident.items() if len(links)==2}
    for i,arc in plans.items():
        path=arc['source_points'];radii=regularize_radii(path)
        for j in range(len(path)):
            required=needed.get((i,j),0)
            if required:
                for k in range(max(0,j-12),min(len(path),j+13)):
                    radii[k]=max(radii[k],required*math.exp(-((k-j)/6)**2))
        if continuous_nodes:
            for endpoint in [0,len(path)-1]:
                target=endpoint_targets.get(tuple(path[endpoint][:2]))
                if target is None:continue
                for j in range(len(path)):
                    distance=abs(path[j][5]-path[endpoint][5]);weight=smoothstep(1-distance/25)
                    radii[j]=max(radii[j],radii[j]*(1-weight)+target*weight)
        if support_core is not None:
            # Least local upper envelope under the radius-slope bound. This
            # preserves measured source support rather than shaving fork wood.
            for order in (range(1,len(radii)),range(len(radii)-2,-1,-1)):
                for j in order:
                    k=j-1 if order.step>0 else j+1
                    radii[j]=max(radii[j],radii[k]-.16*math.dist(path[j][:2],path[k][:2]))
        if continuous_trunk and not arc['tree_arc']:
            # A projected cycle is not automatically a physical attachment.
            # Narrow its far end continuously instead of exposing a flat disk.
            # Source loss, if any, remains visible to the full coverage audit.
            total=path[-1][5]
            for j in range(len(path)):
                remaining=total-path[j][5]
                if remaining<min(12,total*.45):
                    w=smoothstep(remaining/max(.001,min(12,total*.45)))
                    radii[j]=.35+(radii[j]-.35)*w
        vertices=[];faces=[];n=16
        transported=None
        if transport_frames:
            from restart2_tree08_transport import frames
            transported=frames([point(q[0],q[1],q[3]) for q in path],radii)
        for j,(x,y,_,depth,_,_) in enumerate(path):
            # A wide tangent estimate prevents pixel staircase normals from
            # twisting adjacent rings while all source center samples stay fixed.
            before=path[max(0,j-8)];after=path[min(len(path)-1,j+8)];dx,dy=after[0]-before[0],after[1]-before[1];length=max(.001,math.hypot(dx,dy));normal=right*(-dy/length)+down*(dx/length)
            for k in range(n):
                a=math.tau*k/n
                nr,br=(transported[j][0],transported[j][1]) if transported else (normal,ray)
                vertices.append(point(x,y,depth)+nr*(math.cos(a)*radii[j])+br*(math.sin(a)*radii[j]))
        for j in range(len(path)-1):
            for k in range(n):a=j*n+k;b=j*n+(k+1)%n;faces.append((a,b,b+n,a+n))
        faces.extend([tuple(reversed(range(n))),tuple((len(path)-1)*n+k for k in range(n))])
        crossing=not arc['tree_arc'] or any(685<=p[0]<=777 and 185<=p[1]<=315 for p in path)
        sections.append(dict(trace_id=i,vertices=vertices,faces=faces,held_crossing=crossing))
    if transport_frames:return sections,obligations,dict(info,arcs=list(plans.values()),max_depth_step_per_source_length=.6,max_depth_curvature=.018,spatial_depth_blending=False,duplicate_basal_removed=True,parallel_transport_radius_curvature_limit=.6)
    # Reuse only the explicit basal continuation recipe, never its spatial
    # branch depth field; the basal centerline receives zero inherited depth.
    rings=[(557,350,19),(557,360,21),(558,371,23),(559,381,22),(561,390,18),(565,400,12),(568,409,5),(569,414,.4)];vertices=[];faces=[];n=24
    for x,y,r in rings:
        for k in range(n):
            a=math.tau*k/n;vertices.append(point(x,y,0)+right*math.cos(a)*r+ray*math.sin(a)*r)
    for j in range(len(rings)-1):
        for k in range(n):a=j*n+k;b=j*n+(k+1)%n;faces.append((a,b,b+n,a+n))
    faces.extend([tuple(reversed(range(n))),tuple((len(rings)-1)*n+k for k in range(n))]);sections.append(dict(trace_id='inferred-basal-continuation',vertices=vertices,faces=faces,held_crossing=False))
    return sections,obligations,dict(info,arcs=list(plans.values()),max_depth_step_per_source_length=.6,max_depth_curvature=.018,spatial_depth_blending=False)


def assemble_without_remesh(bpy,collection,sections):
    """Assemble exact swept surfaces; no global remesh, smoothing or decimation."""
    vertices=[];faces=[]
    for section in sections:
        offset=len(vertices);vertices.extend(section['vertices']);faces.extend(tuple(offset+i for i in f) for f in section['faces'])
    mesh=bpy.data.meshes.new('Explicit continuous stem and rooted bough surfaces');mesh.from_pydata(vertices,[],faces);mesh.update()
    # Smooth shading changes normals only; source centerlines/radii stay exact.
    for p in mesh.polygons:p.use_smooth=len(p.vertices)==4
    obj=bpy.data.objects.new('Tree08 local sweep correction without global remesh',mesh);collection.objects.link(obj)
    obj['construction_limit']='Branch tubes meet by overlap; only main trunk is one continuous sweep. Junction topology/contact still require verification.'
    return obj
