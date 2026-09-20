"""Close West Tower shell seams and give its revealed landing real thickness."""
import bpy
import bmesh
from mathutils import Vector

def _check(bm):
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    result={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
            'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces),
            'volume':bm.calc_volume(),'faces':len(bm.faces)}
    if result['nonmanifold_edges'] or result['degenerate_faces'] or result['volume']<=0:
        raise ValueError(result)
    return result

def refine():
    working=bpy.data.collections['Derby Working']
    targets={int(o['source_node'][-3:]):o for o in working.objects
             if o.type=='MESH' and not o.hide_render and o.get('source_node') in
             {'building-136','building-239','building-240'}}
    if all(o.get('west_closed_shell_version')==1 for o in targets.values()):
        return {'status':'already-applied'}
    if any(o.get('west_closed_shell_version') for o in targets.values()):
        raise ValueError('Partial West shell recipe')
    report={}
    wall=targets[136]
    if wall.get('west_door_relief_version')!=1:raise ValueError('Apply rooftop doorway recipe first')
    bm=bmesh.new();bm.from_mesh(wall.data)
    # Adjacent collision-derived panels differ by less than a quarter source pixel.
    # Weld their matching seams without changing the measured wall footprint.
    bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.25)
    points=[(285.168,-1521.856,0),(285.168,-1521.856,622.597),
            (270.574,-1557.977,622.597),(270.574,-1557.978,0)]
    vertices=[min(bm.verts,key=lambda v:(wall.matrix_world@v.co-Vector(p)).length) for p in points]
    # This inward panel is concealed by the turret landing solid. Close it down
    # to the existing base; leave the actual open rooftop above the landing.
    bm.faces.new(vertices)
    bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
    report['building-136']=_check(bm)
    wall.data=wall.data.copy();bm.to_mesh(wall.data);bm.free()
    for node,indices,lower,upper in ((239,range(24,33),296.25,304.25),
                                      (240,range(20,26),296.25,467.0)):
        obj=targets[node]
        outline=[obj.matrix_world@obj.data.vertices[i].co for i in indices]
        points=[Vector((p.x,p.y,z)) for z in (lower,upper) for p in outline]
        n=len(outline)
        faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]
        faces.extend((i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n))
        mesh=bpy.data.meshes.new(obj.data.name+' / closed landing assembly')
        inverse=obj.matrix_world.inverted()
        mesh.from_pydata([inverse@p for p in points],[],faces)
        for material in obj.data.materials:mesh.materials.append(material)
        for layer in obj.data.uv_layers:mesh.uv_layers.new(name=layer.name)
        bm=bmesh.new();bm.from_mesh(mesh)
        report[f'building-{node}']=_check(bm)
        bm.to_mesh(mesh);bm.free();obj.data=mesh
        report[f'building-{node}'].update({'bottom':lower,'top':upper,'outline_preserved':True})
    for obj in targets.values():obj['west_closed_shell_version']=1
    return {'changed_nodes':['building-136','building-239','building-240'],
            'parts':report,'landing_thickness_inferred':8,
            'state_contract':'Patch001 receiver IDs and cover geometry unchanged; visible floor and wall crown outlines preserved'}
