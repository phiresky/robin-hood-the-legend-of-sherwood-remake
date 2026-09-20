"""Source-measured East Hall follow-up candidates; require state-aware review.

No automatic publication. Apply in the isolated worker, reproject both covered
and patch-002 receivers, and inspect the assembly before accepting a candidate.
"""
import math

import bmesh
import bpy
from mathutils import Vector

from derby_round2_east_hall import _replace


def section_quads(segments, tolerance=.002):
    """Cap a planar cross-section by paired intervals, preserving hollow gaps.

    Each segment is a pair of world points on x=1586+.055*z. Splitting at every
    endpoint height retains all measured corners. Unlike a convex hull, pairing
    the intersections leaves the space between opposing wall strips open.
    """
    levels=[]
    for z in sorted(p[2] for edge in segments for p in edge):
        if not levels or z-levels[-1]>tolerance:
            levels.append(z)
    quads=[]
    def at(edge,z):
        a,b=edge
        y=a[1]+(b[1]-a[1])*(z-a[2])/(b[2]-a[2])
        return (1586+.055*z,y,z)
    for lo,hi in zip(levels,levels[1:]):
        mid=(lo+hi)/2
        active=[e for e in segments if min(e[0][2],e[1][2])<mid<max(e[0][2],e[1][2])]
        active.sort(key=lambda e:at(e,mid)[1])
        unique=[]
        for edge in active:
            if not unique or abs(at(edge,mid)[1]-at(unique[-1],mid)[1])>tolerance:
                unique.append(edge)
        if len(unique)%2:
            raise ValueError(f'Unpaired wall section at z={mid}: {len(unique)} edges')
        for a,b in zip(unique[::2],unique[1::2]):
            polygon=[at(a,lo),at(b,lo),at(b,hi),at(a,hi)]
            clean=[]
            for p in polygon:
                if not clean or sum((a-b)**2 for a,b in zip(p,clean[-1]))>tolerance**2:
                    clean.append(p)
            if len(clean)>2 and sum((a-b)**2 for a,b in zip(clean[0],clean[-1]))<tolerance**2:
                clean.pop()
            if len(clean)>=3:
                quads.append(clean)
    return quads


def refine_rear_boundary():
    """Clip exterior/interior shells together; preserve the already inset floors."""
    changes=[]
    for node in ('building-185','building-241'):
        objects=[o for o in bpy.data.collections['Derby Working'].all_objects
                 if o.type=='MESH' and o.get('source_node')==node and not o.hide_render]
        if len(objects)!=1:
            raise ValueError(f'Expected one {node}')
        obj=objects[0]
        if obj.get('east_hall_rear_boundary')=='paired-planar-caps-v1':
            changes.append({'source_node':node,'status':'already-refined'})
            continue
        bm=bmesh.new();bm.from_mesh(obj.data)
        before=sum(not e.is_manifold for e in bm.edges)
        matrix=obj.matrix_world
        for v in bm.verts:v.co=matrix@v.co
        # These exported strips contain sub-pixel duplicate corners. Join only
        # those seams before determining paired solid-wall intersections.
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.15)
        cut=bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),
            plane_co=Vector((1586,0,0)),plane_no=Vector((1,0,-.055)),
            dist=.0001,clear_outer=True,clear_inner=False)
        edges=[e for e in cut['geom_cut'] if isinstance(e,bmesh.types.BMEdge) and e.is_boundary]
        segments=[[tuple(v.co) for v in e.verts] for e in edges]
        caps=section_quads(segments)
        for polygon in caps:
            bm.faces.new([bm.verts.new(p) for p in polygon])
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.002)
        # Match cap slab endpoints on neighboring longer cut edges.
        points=[v for v in bm.verts if abs(v.co.x-.055*v.co.z-1586)<.003]
        for edge in list(bm.edges):
            a,b=edge.verts;delta=b.co-a.co
            if delta.length_squared<1e-10:continue
            cuts=[]
            for v in points:
                if v in (a,b):continue
                t=(v.co-a.co).dot(delta)/delta.length_squared
                if 1e-5<t<1-1e-5 and (a.co+t*delta-v.co).length<.002:cuts.append(t)
            previous,current=0.,a
            for t in sorted(set(round(t,8) for t in cuts)):
                _,inserted=bmesh.utils.edge_split(edge,current,(t-previous)/(1-previous))
                previous,current=t,inserted
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.002)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        invalid=sum(f.calc_area()<1e-8 for f in bm.faces)
        if invalid:raise ValueError(f'{node}: {invalid} degenerate faces')
        after=sum(not e.is_manifold for e in bm.edges)
        inverse=matrix.inverted()
        for v in bm.verts:v.co=inverse@v.co
        mesh=obj.data.copy();bm.to_mesh(mesh);bm.free();obj.data=mesh
        obj['east_hall_rear_boundary']='paired-planar-caps-v1'
        changes.append({'source_node':node,'caps':len(caps),'nonmanifold_before':before,
                        'nonmanifold_after':after,'degenerate':invalid})
    return changes


def refine_bay_roof():
    """Replace the broad flat entrance proxy with the observed tile roof.

    Source corners are measured in both covered and revealed imagery. Eave
    elevation retains the old proxy's upper level; the rise follows the Hall's
    perpendicular plan axes, rather than extending an arbitrary flat platform.
    """
    matches = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and o.get('source_node') == 'building-270'
               and not o.hide_render]
    if len(matches) != 1:
        raise ValueError('Expected exactly one upper entrance proxy')
    obj = matches[0]
    tag = 'east-hall-tile-bay-roof-v2'
    obj['part_name'] = 'Tiled entrance bay'
    if obj.get('east_hall_bay_roof') == tag:
        return {'status': 'already-refined'}
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    corners = [(1202,997,328.5), (1229,1015,328.5),
               (1257,980,354.1), (1230,962,354.1)]
    top = [(x, -(y+z*cosine)/sine, z) for x,y,z in corners]
    # The painted entrance walls support the eaves down to the shared entrance
    # landing. Their unseen lower continuation is not extrapolated to ground.
    vertices = top + [(x,y,134.287) for x,y,z in top]
    faces = [(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),
             (2,6,7,3),(3,7,4,0)]
    result = _replace(obj,vertices,faces)
    obj['east_hall_bay_roof'] = tag
    # This is an exterior roof retained when the facade cover is removed.
    obj['east_hall_patch_002_role'] = 'retained exterior bay roof'
    result['source_corners'] = corners
    result['state_evidence'] = ['covered','revealed:patch-002']
    result['doorway_depth'] = 'Painted doorway retained; opening depth not yet reconstructed.'
    return result


def refine_door_recess():
    """Keep the observed wood door closed and recess it behind its stone arch.

    Source rays expose two incorrect overlapping proxies: the old lintel crosses
    the lower panel, while the new upper-bay block extends in front of the whole
    door. Retain the measured212/192 facade plane; bound270 above the recess and
    replace269 with the closed panel and projecting stone surround.
    """
    objects={}
    for node in ('building-269','building-270'):
        found=[o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type=='MESH' and o.get('source_node')==node and not o.hide_render]
        if len(found)!=1:raise ValueError(f'Expected one {node}')
        objects[node]=found[0]
    door,bay=objects['building-269'],objects['building-270']
    tag='east-hall-closed-arched-door-v1'
    if door.get('east_hall_door')==tag:return {'status':'already-refined'}
    if bay.get('east_hall_bay_roof')!='east-hall-tile-bay-roof-v2':
        raise ValueError('Apply the measured entrance bay recipe before its recess')
    if len(bay.data.vertices)!=8:raise ValueError('Reaudit changed entrance bay topology')
    bay.data=bay.data.copy();inverse=bay.matrix_world.inverted()
    for vertex in bay.data.vertices:
        point=bay.matrix_world@vertex.co
        if point.z<235:point.z=235;vertex.co=inverse@point
    bay.data.update()
    normal=Vector((-.7547109,-.6560574,0)).normalized()
    tangent=Vector((-.6560574,.7547109,0)).normalized()
    # Reverse tangent so increasing local u follows increasing source x.
    tangent.negate()
    center=Vector((1244,-2188.274658-(-.7547109)*(1244-1250)/(-.6560574),0))
    def outline(radius,rise,bottom):
        return [(-radius,bottom),(radius,bottom)]+[
            (radius*math.cos(i*math.pi/24),171+rise*math.sin(i*math.pi/24))
            for i in range(25)]
    outer=outline(20,22.5,130.3);inner=outline(15.25,17.5,134.3)
    vertices,faces=[],[]
    def point(u,z,depth):return tuple(center+tangent*u+Vector((0,0,z))+normal*depth)
    for depth in (.3,4.3):
        for contour in (outer,inner):vertices.extend(point(u,z,depth) for u,z in contour)
    count=len(outer)
    for i in range(count):
        j=(i+1)%count
        faces.extend([(i,j,count+j,count+i),
                      (2*count+i,3*count+i,3*count+j,2*count+j),
                      (i,2*count+i,2*count+j,j),
                      (count+i,count+j,3*count+j,3*count+i)])
    start=len(vertices)
    for depth in (.15,.7):vertices.extend(point(u,z,depth) for u,z in inner)
    faces.extend([tuple(start+i for i in range(count)),
                  tuple(start+count+i for i in reversed(range(count)))])
    for i in range(count):
        j=(i+1)%count;faces.append((start+i,start+j,start+count+j,start+count+i))
    result=_replace(door,vertices,faces)
    door['east_hall_door']=tag
    door['part_name']='Closed entrance door and arched stone surround'
    bay['east_hall_door_clearance']=235
    result.update({'closed_panel':True,'surround_depth':4,'panel_recess':3.6,
                   'facade_nodes_preserved':['building-212','building-192'],
                   'upper_bay_bottom':235})
    return result
