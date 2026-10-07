"""One closed west-bank body: wrapped shelves and recessed connecting risers."""
import argparse,bisect,hashlib,json,math,sys
from collections import Counter
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement'
RECIPE=B/'restart2/bank-morphology-trace-v1/recipe.json'
PLAN=B/'restart2/bank-continuous-strata-plan-v1'
OUT=B/'restart2/bank-continuous-strata-v1'
S=math.sin(math.radians(35));C=math.cos(math.radians(35))

def cross(a,b,c):return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])

def triangulate(points):
    indices=list(range(len(points)))
    area=sum(points[i][0]*points[(i+1)%len(points)][1]-points[(i+1)%len(points)][0]*points[i][1] for i in indices)
    if area<0:indices.reverse()
    triangles=[]
    while len(indices)>3:
        for n,b in enumerate(indices):
            a=indices[n-1];c=indices[(n+1)%len(indices)]
            if cross(points[a],points[b],points[c])<=1e-10:continue
            if any(cross(points[a],points[b],points[p])>=-1e-10 and cross(points[b],points[c],points[p])>=-1e-10 and cross(points[c],points[a],points[p])>=-1e-10 for p in indices if p not in (a,b,c)):continue
            triangles.append([a,b,c]);indices.pop(n);break
        else:raise ValueError('Non-simple or degenerate cap polygon')
    triangles.append(indices);return triangles

def interp(points,u):
    lo=points[0][0];width=points[-1][0]-lo;xs=[(p[0]-lo)/width for p in points];i=min(len(points)-2,max(0,bisect.bisect_right(xs,u)-1));t=(u-xs[i])/(xs[i+1]-xs[i])
    return [points[i][j]*(1-t)+points[i+1][j]*t for j in range(3)]

def wrap(front):
    left,right=front[0],front[-1];mid=(left[0]+right[0])/2;back_y=335
    # These six shoulder/back points are inferred; the observed front remains exact.
    return front+[[right[0]+5,right[1]+9/S,right[2]],
        [right[0]+4,-back_y/S,right[2]+1/C],
        [mid,-(back_y-3)/S,(left[2]+right[2])/2+2/C],
        [left[0]-4,-back_y/S,left[2]+1/C],
        [left[0]-7,left[1]+20/S,left[2]],
        [left[0]-5,left[1]+8/S,left[2]]]

def geometry():
    recipe=json.loads(RECIPE.read_text());shelves=recipe['shelves'];us={0.,1.}
    for shelf in shelves:
        pts=[r['upper_world'] for r in shelf['proposed_short_face_rings']];a,z=pts[0][0],pts[-1][0]
        us.update((p[0]-a)/(z-a) for p in pts)
    us=sorted(us);fronts=[];labels=[];exact=[]
    for i,shelf in enumerate(shelves):
        top=[r['upper_world'] for r in shelf['proposed_short_face_rings']];bottom=[r['lower_world'] for r in shelf['proposed_short_face_rings']]
        if i:
            # Recess the connecting face instead of leaving a broad straight ramp
            # behind three independent caps. It is part of the same exterior skin.
            previous=fronts[-1];following=[interp(top,u) for u in us]
            transition=[[.5*(a[0]+b[0]),.5*(a[1]+b[1])+6/S,.5*(a[2]+b[2])] for a,b in zip(previous,following)]
            fronts.append(transition);labels.append('recessed-riser-'+str(i))
        for key,pts in [('upper',top),('lower',bottom)]:
            front=[interp(pts,u) for u in us];fronts.append(front);labels.append(shelf['id']+'-'+key)
            for p in pts:exact.append(min(max(abs(p[j]-q[j]) for j in range(3)) for q in front))
    rings=[wrap(f) for f in fronts]
    # Close onto the original low-level perimeter rather than a cut-off plane.
    level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());original=level['sight_obstacles'][53]['points']
    def base(p):return [p['x'],-p['y']/S,0.]
    a,b=base(original[4]),base(original[3]);bottom=[[a[j]*(1-u)+b[j]*u for j in range(3)] for u in us]
    midpoint={k:(original[1][k]+original[0][k])/2 for k in ['x','y']}
    bottom += [base(original[2]),base(original[1]),base(midpoint),base(original[0]),base(original[6]),base(original[5])]
    rings.append(bottom);labels.append('original-lower-footprint');count=len(rings[0]);assert all(len(r)==count for r in rings)
    vertices=[p for ring in rings for p in ring];faces=[]
    for i in range(len(rings)-1):
        for j in range(count):
            a=i*count+j;b=(i+1)*count+j;c=(i+1)*count+(j+1)%count;d=i*count+(j+1)%count
            faces.extend([[a,b,c],[a,c,d]])
    faces+=triangulate(rings[0]);offset=(len(rings)-1)*count;faces += [[offset+k for k in reversed(t)] for t in triangulate(rings[-1])]
    edges=Counter(tuple(sorted((a,b))) for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(edges.values())=={2};assert max(exact)<1e-9
    areas=[];volume=0.
    for f in faces:
        a,b,c=[vertices[i] for i in f];ab=[b[i]-a[i] for i in range(3)];ac=[c[i]-a[i] for i in range(3)];normal=[ab[1]*ac[2]-ab[2]*ac[1],ab[2]*ac[0]-ab[0]*ac[2],ab[0]*ac[1]-ab[1]*ac[0]];areas.append(math.sqrt(sum(v*v for v in normal))/2)
        volume+=sum(a[i]*normal[i] for i in range(3))/6
    assert min(areas)>1e-8 and volume>1
    return dict(vertices=vertices,faces=faces,ring_labels=labels,ring_size=count,source_trace_max_world_error=max(exact),signed_volume=volume,minimum_triangle_area=min(areas),edge_count=len(edges),recipe_sha256=hashlib.sha256(RECIPE.read_bytes()).hexdigest(),status='CPU closed connected surface; self-intersection and saved model checks pending')

def prepare():
    PLAN.mkdir(exist_ok=False);data=geometry();data.update(dict(scope='Complete visible western bank53 body, not attached caps. Other bank pieces and frozen crest remain unchanged.',
        controls=['Three source-traced front bands wrap into continuous shoulders/back surfaces.','Recessed sloping risers connect bands in a single closed exterior mesh; no Boolean core or broad front backing remains.','Original lower footprint closes the solid; full enclosed volume remains behind strata.'],
        runtime_guards=['Closed one-component manifold; no degeneracies or self intersections.','3446 source first-hit seed assignment and neighbor-owned ray guards.','Unchanged fixed crest and context94–97, gameplay metadata exact.','Saved original-native camera first, actual8/solid8 and west contact self-review.'],
        limits=['Source-supported front coordinates are exact; shoulder wrapping, recess depth, rear closure and heights remain inferred.','No source ownership approval or canonical mutation.','CPU volume does not certify a non-self-intersecting saved mesh; next Blender run must check it.']))
    (PLAN/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print(PLAN/'geometry.json')

def run():
    import bpy,bmesh
    sys.path.insert(0,str(Path(__file__).parent));import restart2_bank_full_v1 as base
    data=json.loads((PLAN/'geometry.json').read_text());assert data['recipe_sha256']==hashlib.sha256(RECIPE.read_bytes()).hexdigest();original=base.mesh_object;base.O=OUT
    def build(name,points,scene):
        if name!='Candidate bank 53':return original(name,points,scene)
        mesh=bpy.data.meshes.new('Continuous wrapped western strata');mesh.from_pydata(data['vertices'],[],data['faces']);mesh.update();obj=bpy.data.objects.new(name,mesh);scene.collection.objects.link(obj)
        bm=bmesh.new();bm.from_mesh(mesh);assert all(e.is_manifold for e in bm.edges) and all(f.calc_area()>1e-8 for f in bm.faces);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert bm.calc_volume()>1;bm.to_mesh(mesh);bm.free();return obj
    base.mesh_object=build;base.main();(OUT/'morphology-plan.json').write_text(json.dumps(data,indent=2)+'\n')
    assert sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file())<32*1024**2

if __name__=='__main__':
    args=sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:];p=argparse.ArgumentParser();p.add_argument('--prepare',action='store_true');a=p.parse_args(args)
    prepare() if a.prepare else run()
