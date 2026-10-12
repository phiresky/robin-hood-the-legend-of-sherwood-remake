"""Isolated north tower revision from recorded artwork corners, in world space."""
import sys
sys.path[:0] = ['/usr/lib/python3.14','/usr/lib/python3.14/lib-dynload','/usr/lib/python3.14/site-packages']
from pathlib import Path
import json, math, hashlib
import bpy, bmesh
from mathutils import Matrix, Vector

ROOT=Path(__file__).resolve().parents[2]
W=ROOT/'level-editor/work/needs-review/derby-refinement/round-2/assets/derby-great-keep'
N=W/'component-followups/north'
OUT=N/'pixel-trace-v4'
S=math.sin(math.radians(35)); C=math.cos(math.radians(35))
NODES={f'building-{i:03}' for i in [163,164,165,166,171,172,173,174,175]}
# Each sequence follows one visible cap arris across the image. Duplicate X
# records are genuine vertical rises/falls, not points fitted to the old mesh.
FRONT=[(922.47,273),(933,286),(933,306),(950,300),(950,291),
       (966,295),(976.31,297),(983,293),(983,314),(997,309),(997,290),
       (1017,285),(1017,306),(1033,301),(1033,284),(1039.09,282),
       (1049,278),(1049,296),(1060,289),(1060,269),
       (1079,259),(1079,280),(1090,272),(1090,252),
       (1106,244),(1106,262),(1116.27,256)]
LEFT=[(912.03,230),(920,226),(920,240),(929,235),(929,214),
      (940,208),(940,223),(950,218),(950,196),(960,188),
      (960,210),(971,203),(971,178),(973.06,177)]
REAR=[(1029.94,163),(1042,169),(1042,193),(1055,198),
      (1055,179),(1065,181),(1076,184),(1076,205),(1088,210),
      (1088,193),(1100.29,197)]

def visible(node):
    return next(o for o in bpy.data.collections['Derby Working'].objects
                if o.type=='MESH' and o.get('source_node')==node and not o.hide_render)

def geom(o):
    return hashlib.sha256(repr(([tuple(v.co) for v in o.data.vertices],
        [tuple(p.vertices) for p in o.data.polygons],list(map(tuple,o.matrix_world)))).encode()).hexdigest()

def new_mesh(obj,vertices,faces):
    mesh=bpy.data.meshes.new(obj.name+' traced mesh');mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.001)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj.data=mesh;obj.matrix_world=Matrix.Identity(4)

def interpolate(points,x,side='right'):
    for i in range(len(points)-1):
        a,b=points[i:i+2]
        if a[0]<=x<=b[0] and b[0]>a[0]:
            if x==b[0] and side=='right' and i+2<len(points) and points[i+2][0]==x:continue
            return a[1]+(b[1]-a[1])*(x-a[0])/(b[0]-a[0])
    return min(points,key=lambda p:abs(p[0]-x))[1]

def wall_span(a,b,trace,depth,base,vertices,faces,record):
    # Each straight wall span is a closed extrusion of the traced top profile.
    if a[0]>b[0]:a,b=b,a
    dx=b[0]-a[0];dy=b[1]-a[1]
    if abs(dx)<1e-6:return
    def world(p):
        x,py=p;y=a[1]+dy*(x-a[0])/dx
        return (x,y,(-py-y*S)/C)
    points=[(a[0],interpolate(trace,a[0],'right'))]
    points += [p for p in trace if a[0]<p[0]<b[0]]
    points += [(b[0],interpolate(trace,b[0],'left'))]
    upper=[world(p) for p in points]
    loop=[(a[0],a[1],base)]+upper+[(b[0],b[1],base)]
    offset=Vector(depth);start=len(vertices);n=len(loop)
    vertices.extend(loop);vertices.extend(tuple(Vector(p)+offset) for p in loop)
    faces.extend([tuple(start+i for i in range(n-1,-1,-1)),tuple(start+n+i for i in range(n))])
    faces.extend((start+i,start+(i+1)%n,start+n+(i+1)%n,start+n+i) for i in range(n))
    record.extend({'pixel':list(p),'world':list(w),'projection_error':math.hypot(w[0]-p[0],-w[1]*S-w[2]*C-p[1])} for p,w in zip(points,upper))

def revise():
    OUT.mkdir(parents=True,exist_ok=True)
    col=bpy.data.collections['Derby Working']; before={o.name:geom(o) for o in col.objects if o.type=='MESH'}
    # Withdraw the erroneous v3 world-frame door and restore the valid source
    # opening. The extra 7 units inset is along positive Y (into the turret).
    door=next(o for o in col.objects if o.get('north_spire_door_backing'))
    outline=[(-12,835),(12,835),(12,875)]+[(12*math.cos(i*math.pi/16),875+12*math.sin(i*math.pi/16)) for i in range(1,17)]
    n=len(outline);v=[(1003.1+x,y,z) for y in (-1562.9,-1561.4) for x,z in outline]
    f=[tuple(range(n-1,-1,-1)),tuple(range(n,2*n))]+[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    new_mesh(door,v,f)
    records={};obj=visible('building-164')
    bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.transform(bm,matrix=obj.matrix_world,verts=list(bm.verts))
    bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,835),plane_no=(0,0,1),clear_outer=True,clear_inner=False)
    bm.verts.ensure_lookup_table();bm.verts.index_update()
    v=[tuple(p.co) for p in bm.verts];f=[tuple(p.index for p in face.verts) for face in bm.faces];bm.free();rec=[]
    outer=[(912.03,-1662.92),(912.02,-1724.46),(922.47,-1757.62),(976.31,-1797.83),(1039.09,-1773.68),(1116.27,-1665.62)]
    # Existing lower-wall footprint is retained. Trace applies to the exposed
    # three front spans; left return is below the pitched connector.
    for a,b in zip(outer,outer[1:]):
        if abs(b[0]-a[0])<.1:
            # This concealed return is copied from the baseline below.
            continue
        dy=b[1]-a[1];dx=b[0]-a[0];length=math.hypot(dx,dy)
        wall_span(a,b,FRONT,(-dy/length*10,dx/length*10,0),835,v,f,rec)
    # Rear-left facade climbs toward the spire; source pixels give its notches.
    wall_span((912.03,-1662.92),(973.06,-1572.44),LEFT,(4,-14,0),835,v,f,rec)
    # Existing hidden return is closed as a vertical masonry panel.
    j=len(v);v.extend([(912.03,-1662.92,835),(912.02,-1724.46,835),(912.02,-1724.46,860),(912.03,-1662.92,860),
                     (922.03,-1662.92,835),(922.02,-1724.46,835),(922.02,-1724.46,860),(922.03,-1662.92,860)])
    f.extend(tuple(j+i for i in face) for face in [(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
    new_mesh(obj,v,f);records['building-164']=rec
    obj=visible('building-174');v=[];f=[];rec=[]
    wall_span((1029.94,-1569.78),(1100.29,-1615.34),REAR,(3.37,5.21,0),835,v,f,rec)
    new_mesh(obj,v,f);records['building-174']=rec
    # Round only the outlook's top ring, preserving its detailed lower corbels.
    obj=visible('building-175');v=[];f=[]
    for p in obj.data.polygons:
        pts=[obj.matrix_world@obj.data.vertices[i].co for i in p.vertices]
        if max(p.z for p in pts)>835.02:continue
        j=len(v);v.extend(tuple(p) for p in pts);f.append(tuple(range(j,len(v))))
    cx,cy=1127.9,-1630.6;ro,ri=35.8,25.2;segments=64;j=len(v)
    for radius,z in [(ro,835),(ro,862.5),(ri,862.5),(ri,842.25)]:
        for i in range(segments):
            a=i*2*math.pi/segments;v.append((cx+radius*math.cos(a),cy+radius*math.sin(a),z))
    for ring in range(3):
        for i in range(segments):f.append((j+ring*segments+i,j+ring*segments+(i+1)%segments,j+(ring+1)*segments+(i+1)%segments,j+(ring+1)*segments+i))
    f.append(tuple(j+3*segments+i for i in range(segments-1,-1,-1)))
    new_mesh(obj,v,f)
    after={o.name:geom(o) for o in col.objects if o.type=='MESH'}
    changed=[n for n in before if before[n]!=after.get(n)]
    assert len(changed)==4,changed
    # No component in North Tower owns an interior patch receiver. Covered and
    # revealed full-map sources are audited separately by the packet builder.
    result={'source_trace':{'front':FRONT,'left':LEFT,'rear':REAR},'records':records,
            'changed_working_meshes':changed,'outside_working_meshes_unchanged':True,
            'door_world_bounds':[[min((door.matrix_world@p.co)[i] for p in door.data.vertices),max((door.matrix_world@p.co)[i] for p in door.data.vertices)] for i in range(3)],
            'outlook_segments':segments,'pixel_uncertainty':2,'sun_elevation':48,
            'limitations':['Hand-selected cap arrises have approximately 2 source-pixel uncertainty.','Hidden adjoining wall returns remain inferred.','Projection residual tests construction against selected pixels, not independent truth.']}
    (OUT/'trace.json').write_text(json.dumps(result,indent=2))
    close_retained_bodies()
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'))
    return result

def review():
    sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'refinement/blender'))
    from refinement_review import render_review
    from source_projection_bake import bake
    from review_sunlight import configuration
    frame=json.loads((N/'revision-v3/approval-v2/views.json').read_text());frame['lighting']=configuration()
    for o in bpy.data.collections['Derby Working'].objects:
        if o.get('source_node') in NODES:o['asset_group']='derby-keep-north-tower'
    manifest=N/'source-masks.json'
    bake('Derby',frame['source_image'],OUT/'bake.json',receiver_nodes=sorted(NODES),projection_label='exterior',preserve_authored=False,source_mask_manifest=manifest)
    render_review(OUT/'modified-v3',scene_name='Derby Refinement',collection_name='Derby Working',asset_id='derby-keep-north-tower',source_path=frame['source_image'],frame_manifest=frame,projection_layers=frame['projection_layers'],source_mask_manifest=manifest,lighting=configuration())
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'))

def close_retained_bodies():
    """Cap clipping planes and inherited lower ends without changing the trace."""
    rows={}
    for node in ['building-164','building-175']:
        o=visible(node);bm=bmesh.new();bm.from_mesh(o.data)
        def counts():
            return {'boundary':sum(e.is_boundary for e in bm.edges),
                    'nonmanifold':sum(not e.is_manifold for e in bm.edges),
                    'faces':len(bm.faces)}
        before=counts()
        # Outlook clipping left three 0.011-unit slivers. Welding those does
        # not move traced parapet corners (which belong to a different mesh).
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.02 if node=='building-175' else .001)
        wire=[e for e in bm.edges if not e.link_faces]
        if wire:bmesh.ops.delete(bm,geom=wire,context='EDGES')
        junctions=[e for e in bm.edges if not e.is_manifold and not e.is_boundary]
        if junctions:bmesh.ops.split_edges(bm,edges=junctions)
        caps=bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)['faces']
        cap_info=[{'area':f.calc_area(),'world_bounds':[[min((o.matrix_world@v.co)[i] for v in f.verts),max((o.matrix_world@v.co)[i] for v in f.verts)] for i in range(3)]} for f in caps]
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        after=counts();assert after['boundary']==after['nonmanifold']==0,(node,after)
        assert all(f.calc_area()>1e-6 for f in bm.faces),node
        bm.to_mesh(o.data);bm.free();o.data.update()
        rows[node]={'before':before,'after':after,'closure_faces':cap_info}
    (OUT/'closure-audit.json').write_text(json.dumps(rows,indent=2))
    # Undo the withdrawn draft's edit to the hidden baseline copy as well.
    # Working terrace geometry is distinct and is deliberately untouched.
    target=bpy.data.objects['building-163']
    assert target not in list(bpy.data.collections['Derby Working'].objects)
    with bpy.data.libraries.load(str(W/'component-followup-source.blend'),link=False) as (src,dst):
        dst.objects=['building-163']
    restored=dst.objects[0];target.data=restored.data
    bpy.data.objects.remove(restored,do_unlink=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'))
    return rows

def audit():
    from mathutils.bvhtree import BVHTree
    data=json.loads((OUT/'trace.json').read_text());rows=[];meshes={};topology={}
    for node in ['building-164','building-174','building-175','building-173']:
        o=visible(node);o.data.calc_loop_triangles()
        vertices=[o.matrix_world@v.co for v in o.data.vertices]
        tris=[tuple(t.vertices) for t in o.data.loop_triangles]
        tree=BVHTree.FromPolygons(vertices,tris,all_triangles=True)
        meshrow={'vertices':[list(v) for v in vertices],'edges':[list(e.vertices) for e in o.data.edges]}
        meshes[node]=meshrow
        bm=bmesh.new();bm.from_mesh(o.data)
        topology[node]={'faces':len(bm.faces),'boundary_edges':sum(e.is_boundary for e in bm.edges),'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),'zero_area_faces':sum(f.calc_area()<1e-6 for f in bm.faces)}
        bm.free()
        for rec in data['records'].get(node,[]):
            p=Vector(rec['world']);nearest=min(vertices,key=lambda v:(v-p).length)
            px=(nearest.x,-nearest.y*S-nearest.z*C)
            expected=rec['pixel'];rows.append({'node':node,'expected':expected,'actual':list(px),'error':math.dist(px,expected)})
    result={'samples':rows,'max_saved_mesh_corner_error':max(r['error'] for r in rows),'mean_saved_mesh_corner_error':sum(r['error'] for r in rows)/len(rows),
            'meaning':'Saved mesh vertices reprojected independently. This checks fitting, not the manual identification of artwork corners.',
            'working_meshes':meshes,'topology':topology,
            'topology_limit':'Revised main wall, rear curtain and outlook each have no boundary or nonmanifold edges. They contain separately closed adjoining solids; this is not a Boolean-unified building.'}
    (OUT/'mesh-audit.json').write_text(json.dumps(result,indent=2))

if __name__=='__main__':
    if '--review' in sys.argv:review()
    else:revise()
