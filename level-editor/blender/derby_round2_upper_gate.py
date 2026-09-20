"""Round gatehouse tower shells and model the covered facade's shallow arch."""
import math
import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-upper-gatehouse'
TAG = 'upper-gate-flanking-arcs-v2'
# Corresponding outer/inner paths run between the wall junctions. The open
# section remains open; smoothing does not turn these shells into full cylinders.
PATHS = {
    118: (
        [(418.0074,-2668.6499),(408.6833,-2680.6831),(418.91,-2715.95),
         (445.96,-2727.44),(477.3197,-2717.54)],
        [(429.3232,-2668.6499),(420.50,-2685.84),(428.23,-2706.84),
         (448.0,-2713.97),(470.94,-2704.49)]),
    125: (
        [(848.3536,-2653.34),(876.59,-2657.05),(905.58,-2643.54),
         (916.3171,-2613.81),(903.20,-2585.34),(876.4,-2575.66)],
        [(850.96,-2646.13),(874.63,-2645.30),(900.98,-2635.18),
         (908.33,-2614.22),(897.90,-2594.52),(872.68,-2585.98)]),
    258: (
        [(724.4075,-2777.7991),(746.0742,-2803.9509),
         (773.5742,-2806.1301),(787.8973,-2780.4097),
         (782.6371,-2741.6609),(755.8322,-2730.3423)],
        [(738.1575,-2776.3464),(747.3242,-2793.0542),
         (770.3581,-2791.5706),(781.0742,-2777.0728),
         (775.6575,-2747.2888),(756.4909,-2742.9302)]),
}


def _curve(points, samples=8):
    """Centripetal Catmull–Rom follows the anchors without polygonal corners."""
    points=[Vector(p) for p in points]
    extended=[2*points[0]-points[1],*points,2*points[-1]-points[-2]]
    result=[]
    for i in range(len(points)-1):
        a,b,c,d=extended[i:i+4]
        times=[0]
        for p,q in zip((a,b,c),(b,c,d)):
            times.append(times[-1]+math.sqrt((q-p).length))
        t0,t1,t2,t3=times
        for step in range(samples):
            t=t1+(t2-t1)*step/samples
            x=(t1-t)/(t1-t0)*a+(t-t0)/(t1-t0)*b
            y=(t2-t)/(t2-t1)*b+(t-t1)/(t2-t1)*c
            z=(t3-t)/(t3-t2)*c+(t-t2)/(t3-t2)*d
            xy=(t2-t)/(t2-t0)*x+(t-t0)/(t2-t0)*y
            yz=(t3-t)/(t3-t1)*y+(t-t1)/(t3-t1)*z
            result.append((t2-t)/(t2-t1)*xy+(t-t1)/(t2-t1)*yz)
    return result+[points[-1]]


def refine():
    report=[]
    for number,(outer,inner) in PATHS.items():
        matches=[o for o in bpy.data.collections['Derby Working'].objects
                 if o.type=='MESH' and not o.hide_render
                 and o.get('source_node')==f'building-{number}'
                 and o.get('asset_group')==ASSET]
        if len(matches)!=1:raise ValueError(f'Expected single tower shell {number}')
        obj=matches[0]
        if obj.get('round2_upper_gate')==TAG:
            report.append({'node':obj['source_node'],'already_applied':True});continue
        world=[obj.matrix_world@v.co for v in obj.data.vertices]
        bottom,top=min(p.z for p in world),max(p.z for p in world)
        # Snap rounded anchor literals to the existing polygon endpoints.
        outer=[min(world,key=lambda p:(p.x-x)**2+(p.y-y)**2).xy.copy() for x,y in outer]
        inner=[min(world,key=lambda p:(p.x-x)**2+(p.y-y)**2).xy.copy() for x,y in inner]
        outside,inside=_curve(outer),_curve(inner)
        outline=outside+list(reversed(inside));n=len(outline)
        inv=obj.matrix_world.inverted()
        vertices=[inv@Vector((p.x,p.y,z)) for z in (bottom,top) for p in outline]
        faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]
        faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
        mesh=bpy.data.meshes.new(obj.name+' / curved shell')
        mesh.from_pydata(vertices,[],faces);mesh.update()
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bmesh.ops.triangulate(bm,faces=[f for f in bm.faces if len(f.verts)>4])
        bad=sum(not e.is_manifold for e in bm.edges)
        degenerate=sum(f.calc_area()<1e-7 for f in bm.faces)
        if bad or degenerate:raise ValueError((obj.name,bad,degenerate))
        bm.to_mesh(mesh);bm.free()
        # Neutral fallback is temporary: the worker projection pass reassigns
        # reference texels against the complete scene before saving the model.
        material=bpy.data.materials.get('Upper gate round-two neutral')
        if material is None:
            material=bpy.data.materials.new('Upper gate round-two neutral')
            material.diffuse_color=(.3,.3,.3,1)
        mesh.materials.append(material)
        mesh.uv_layers.new(name='UVMap')
        mesh.attributes.new('reprojection_fallback_material','INT','FACE')
        obj.data=mesh;obj['round2_upper_gate']=TAG
        report.append({'node':obj['source_node'],'vertices':len(mesh.vertices),
                       'faces':len(mesh.polygons),'nonmanifold_edges':bad,
                       'degenerate_faces':degenerate,'retained_height':[bottom,top],
                       'outer_samples':len(outside),'inner_samples':len(inside)})
    return {'asset':ASSET,'recipe':TAG,'objects':report}


def recess_front_panel():
    """Carve the shaded upper arch as a shallow recess in its removable panel.

    The panel is a reveal cover, not an open lower passage. Keep masonry behind
    the recess so it does not expose the room in the covered state.
    """
    obj=next(o for o in bpy.data.collections['Derby Working'].objects
             if o.type=='MESH' and not o.hide_render
             and o.get('source_node')=='building-257')
    if obj.get('round2_upper_arch_recess'):
        return {'already_applied':True}
    # Interior edge traced from the covered facade; deliberately shallow depth
    # reconstructs the visible shadow without assuming an unseen opening.
    trace=[(622,1452),(622,1416),(626,1406),(634,1398),(643,1392),
           (654,1388),(665,1388),(677,1390),(688,1395),(697,1402),
           (703,1411),(703,1452)]
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
    vertices=[]
    for depth in (-2,4):
        for x,screen_y in trace:
            y=-2791.9+(x-581.0)*(14.4/149.8)
            vertices.append((x,y+depth,(-y*s-screen_y)/c))
    n=len(trace)
    faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    mesh=bpy.data.meshes.new('Upper panel shallow arch cutter')
    mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    cutter=bpy.data.objects.new(mesh.name,mesh)
    bpy.context.scene.collection.objects.link(cutter)
    try:
        bm=bmesh.new();bm.from_mesh(obj.data)
        before_volume=abs(bm.calc_volume(signed=True));bm.free()
        modifier=obj.modifiers.new('Shallow covered arch recess','BOOLEAN')
        modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
        bpy.context.view_layer.objects.active=obj
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        bm=bmesh.new();bm.from_mesh(obj.data)
        bad=sum(not e.is_manifold for e in bm.edges)
        degenerate=sum(f.calc_area()<1e-7 for f in bm.faces)
        after_volume=abs(bm.calc_volume(signed=True))
        bm.free()
        if bad or degenerate:raise ValueError(('arch recess topology',bad,degenerate))
        if not 0 < before_volume-after_volume < before_volume*.3:
            raise ValueError(('arch recess did not remove bounded volume',before_volume,after_volume))
        obj['round2_upper_arch_recess']=True
        return {'node':'building-257','depth':4,'nonmanifold_edges':bad,
                'degenerate_faces':degenerate,'depth_is_inferred':True}
    finally:
        bpy.data.objects.remove(cutter,do_unlink=True);bpy.data.meshes.remove(mesh)
