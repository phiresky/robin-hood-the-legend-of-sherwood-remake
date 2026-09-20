"""Measured Great Keep rooftop parapet correction.

The narrow rear crenellated strip belongs to the north tower terrace. Its
collision extrusion previously continued 835 units below its supporting roof,
making an isolated vertical blade visible from the side and rear.
"""
import bpy
import bmesh
import math
from mathutils import Vector, Matrix


def refine():
    matches = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == 'building-174'
               and o.get('asset_group') == 'derby-great-keep']
    if len(matches) != 1:
        raise ValueError('Expected one visible Great Keep rear parapet')
    obj = matches[0]
    before = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
    inverse = obj.matrix_world.inverted()
    changed = 0
    for vertex in obj.data.vertices:
        world = obj.matrix_world @ vertex.co
        if world.z < 835.0:
            world.z = 835.0
            vertex.co = inverse @ world
            changed += 1
    obj.data.update()
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.free()
    if bad_edges or bad_faces:
        raise ValueError('Rear parapet must remain a closed nondegenerate shell')
    obj['round2_keep_recipe'] = 'north-roof-parapet-base-v1'
    return {'asset': 'derby-great-keep', 'changed_nodes': ['building-174'],
            'changed_vertices': changed, 'old_bottom': before, 'new_bottom': 835.0,
            'faces': len(obj.data.polygons), 'nonmanifold_edges': bad_edges,
            'degenerate_faces': bad_faces,
            'evidence': 'Native scenery mask144 and original roof terrace image'}


def refine_fireplace():
    """Build the painted hall fireplace as a shallow hood and open hearth.

    Four measured hood corners determine front width and slope. The existing
    rear room wall and floor anchor the volume; hidden depth is conservative.
    This component belongs to the revealed hall receiver, never its cover.
    """
    working = bpy.data.collections['Derby Working']
    tag = 'great-keep-hall-fireplace-v1'
    existing=[o for o in working.objects if o.get('round2_keep_recipe') == tag]
    if existing:
        for obj in existing:obj['projection_component']='hall-fireplace'
        return {'status': 'already-applied'}
    source = next(o for o in working.objects if o.type == 'MESH'
                  and not o.hide_render and o.get('source_node') == 'building-227')
    normal = Vector((-.544638991, -.83867067, 0))
    tangent = Vector((.83867067, -.544638991, 0))
    wall = Vector((970, -1644.329, 0))
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def source_point(x, y, depth):
        world_y = wall.y + (depth - normal.x * (x-wall.x)) / normal.y
        return Vector((x, world_y, (-y-world_y*sine)/cosine))

    top = [source_point(946, 661, 3), source_point(995, 679, 3)]
    bottom = [source_point(932, 719, 28), source_point(981, 736, 28)]
    for ring in (top, bottom):
        level = sum(v.z for v in ring)/2
        for point in ring: point.z = level
    vertices, faces = [], []

    def prism(front, depth):
        base = len(vertices)
        vertices.extend(front)
        vertices.extend(p-normal*depth for p in front)
        faces.extend(tuple(base+i for i in f) for f in
                     ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),
                      (2,6,7,3),(3,7,4,0)))

    # Sloping hood: rear points land on the existing masonry wall.
    front = [bottom[0], bottom[1], top[1], top[0]]
    base = len(vertices)
    vertices.extend(front)
    vertices.extend([bottom[0]-normal*28, bottom[1]-normal*28,
                     top[1]-normal*3, top[0]-normal*3])
    faces.extend(tuple(base+i for i in f) for f in
                 ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),
                  (2,6,7,3),(3,7,4,0)))
    z_bottom = bottom[0].z
    left, right = bottom[0]-tangent*1.5+normal*1.5, bottom[1]+tangent*1.5+normal*1.5
    prism([left-Vector((0,0,5)), right-Vector((0,0,5)), right, left], 30)
    # Side jambs leave an actual empty hearth opening backed by the room wall.
    for a, b in ((bottom[0],bottom[0]+tangent*4),
                 (bottom[1]-tangent*4,bottom[1])):
        low_a, low_b = a.copy(), b.copy()
        low_a.z = low_b.z = 219.75
        high_a, high_b = a.copy(), b.copy()
        high_a.z = high_b.z = z_bottom-5
        prism([low_a,low_b,high_b,high_a],28)
    mesh = bpy.data.meshes.new('Great Keep / revealed hall fireplace mesh')
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges=sum(not e.is_manifold for e in bm.edges)
    bad_faces=sum(f.calc_area()<1e-8 for f in bm.faces)
    bm.to_mesh(mesh); bm.free()
    if bad_edges or bad_faces: raise ValueError('Invalid fireplace component')
    neutral=bpy.data.materials.get('Great Keep / fireplace unknown')
    if neutral is None:
        neutral=bpy.data.materials.new('Great Keep / fireplace unknown')
        neutral.diffuse_color=(.25,.25,.25,1)
    mesh.materials.append(neutral)
    for original in source.data.uv_layers:
        layer=mesh.uv_layers.new(name=original.name)
        for uv in layer.data: uv.uv=(.5,.5)
    obj=bpy.data.objects.new('Great Keep / Hall revealed fireplace',mesh)
    working.objects.link(obj); obj.parent=source.parent; obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith('reprojection_'): obj[key]=source[key]
    obj['round2_keep_recipe']=tag
    obj['round2_keep_component']='hall-fireplace'
    obj['projection_component']='hall-fireplace'
    return {'source_node':'building-227','new_mesh':obj.name,'faces':len(faces),
            'nonmanifold_edges':bad_edges,'degenerate_faces':bad_faces,
            'floor':219.75,'mantel':z_bottom,'hood_top':top[0].z,
            'inferred_depth':28,'source_state':'patch-000 revealed',
            'source_hood_corners':[[946,661],[995,679],[981,736],[932,719]]}


def refine_fireplace_arch():
    """Restore the shallow curved hearth opening beneath the measured hood.

    The existing jambs determine the span and rear depth. The source shows a
    depressed arch rising about nineteen units from its spring to the crown.
    This adds the stone spandrels; the opening and firelit inner wall stay open.
    """
    working=bpy.data.collections['Derby Working']
    tag='great-keep-hall-fireplace-arch-v1'
    if any(o.get('round2_keep_recipe')==tag for o in working.objects):
        return {'status':'already-applied'}
    source=next(o for o in working.objects if o.get('projection_component')=='hall-fireplace')
    if source.get('round2_keep_recipe')!='great-keep-hall-fireplace-v1' or len(source.data.vertices)!=32:
        raise ValueError('Fireplace arch requires the reviewed four-solid fireplace topology')
    normal=Vector((-.544638991,-.83867067,0))
    left=source.data.vertices[17].co.copy()
    right=source.data.vertices[24].co.copy()
    # Vertex17 is the left jamb's inner lower corner;24 is the right one's.
    top=source.data.vertices[18].co.z
    spring=253.0;crown=272.0;segments=12;vertices=[];faces=[]
    for rear in (False,True):
        for i in range(segments+1):
            t=i/segments
            point=left.lerp(right,t)-(normal*28 if rear else Vector((0,0,0)))
            lower=point.copy();lower.z=spring+(crown-spring)*math.sqrt(max(0,1-(2*t-1)**2))
            upper=point.copy();upper.z=top
            vertices.extend((lower,upper))
    stride=2*(segments+1)
    for i in range(segments):
        a=2*i;b=a+2
        faces.extend(((a,b,b+1,a+1),(a+stride+1,b+stride+1,b+stride,a+stride),
                      (a,a+stride,b+stride,b),(a+1,b+1,b+stride+1,a+stride+1)))
    faces.extend(((0,1,stride+1,stride),(stride-2,2*stride-2,2*stride-1,stride-1)))
    mesh=bpy.data.meshes.new('Great Keep / fireplace curved stone arch mesh')
    mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    invalid=sum(not e.is_manifold for e in bm.edges)
    degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
    bm.to_mesh(mesh);bm.free()
    if invalid or degenerate:raise ValueError('Invalid fireplace stone arch')
    mesh.materials.append(bpy.data.materials['Great Keep / fireplace unknown'])
    for original in source.data.uv_layers:
        layer=mesh.uv_layers.new(name=original.name)
        for uv in layer.data:uv.uv=(.5,.5)
    obj=bpy.data.objects.new('Great Keep / Hall fireplace curved stone arch',mesh)
    working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=source.matrix_world.copy()
    for key in source.keys():
        if not key.startswith('reprojection_'):obj[key]=source[key]
    obj['round2_keep_recipe']=tag
    obj['round2_keep_component']='hall-fireplace-arch'
    obj['projection_component']='hall-fireplace-arch'
    return {'object':obj.name,'faces':len(faces),'nonmanifold_edges':invalid,
            'degenerate_faces':degenerate,'spring_height':spring,'crown_height':crown,'top':top}


def refine_front_oriel():
    """Model the source-visible projecting facade window, tile cap and corbels."""
    working=bpy.data.collections['Derby Working'];tag='great-keep-front-oriel-v1'
    if any(o.get('round2_keep_recipe')==tag for o in working.objects):
        return {'status':'already-applied'}
    source=next(o for o in working.objects if o.type=='MESH' and not o.hide_render and o.get('source_node')=='building-179' and not o.get('round2_keep_recipe'))
    normal=Vector((-.5519364476,-.8338860869,0));anchor=Vector((814,-2094.144,0))
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    def point(x,y,depth):
        wy=anchor.y+(depth-normal.x*(x-anchor.x))/normal.y
        return Vector((x,wy,(-y-wy*sine)/cosine))
    def level(x,y,depth,z):
        p=point(x,y,depth);p.z=z;return p
    bottom=sum(point(x,y,20).z for x,y in ((806,842),(829,850)))/2
    cap_front=sum(point(x,y,22).z for x,y in ((803,807),(829,816)))/2
    cap_rear=sum(point(x,y,0).z for x,y in ((814,789),(839,797)))/2
    top=cap_front-3+(cap_rear-cap_front)*2/23
    opening_bottom=sum(point(x,y,20).z for x,y in ((814,833),(820,835)))/2
    opening_top=sum(point(x,y,20).z for x,y in ((814,823),(820,825)))/2
    outer=[level(x,0,20,z) for x,z in ((806,bottom),(829,bottom),(829,top),(806,top))]
    inner=[level(x,0,20,z) for x,z in ((814,opening_bottom),(820,opening_bottom),(820,opening_top),(814,opening_top))]
    vertices=outer+inner+[p-normal*21 for p in outer]+[p-normal*21 for p in inner];faces=[]
    for index in (10,11):vertices[index].z=cap_rear-3
    for i in range(4):
        j=(i+1)%4
        faces.extend(((i,j,j+4,i+4),(i+8,i+12,j+12,j+8),
                      (i,i+8,j+8,j),(i+4,j+4,j+12,i+12)))
    def solid(front,rear):
        base=len(vertices);vertices.extend(front+rear)
        faces.extend(tuple(base+i for i in f) for f in ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)))
    a,b=[level(x,0,22,cap_front) for x in (803,831)]
    rear_a,rear_b=a-normal*23,b-normal*23;rear_a.z=rear_b.z=cap_rear
    solid([a-Vector((0,0,3)),b-Vector((0,0,3)),b,a],
          [rear_a-Vector((0,0,3)),rear_b-Vector((0,0,3)),rear_b,rear_a])
    for x in (808.5,817.5,826.5):
        high=[level(xx,0,18,bottom+.5) for xx in (x-1.6,x+1.6)]
        low=[level(xx,0,14,bottom-13) for xx in (x-1.6,x+1.6)]
        solid([low[0],low[1],high[1],high[0]],
              [low[0]-normal*15,low[1]-normal*15,high[1]-normal*19,high[0]-normal*19])
    mesh=bpy.data.meshes.new('Great Keep / facade oriel mesh');mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    invalid=sum(not e.is_manifold for e in bm.edges);degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
    bm.to_mesh(mesh);bm.free()
    if invalid or degenerate:raise ValueError('Invalid facade oriel')
    mesh.materials.append(bpy.data.materials['Great Keep / fireplace unknown'])
    for original in source.data.uv_layers:
        layer=mesh.uv_layers.new(name=original.name)
        for uv in layer.data:uv.uv=(.5,.5)
    obj=bpy.data.objects.new('Great Keep / Hall projecting window and tile cap',mesh)
    working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith('reprojection_'):obj[key]=source[key]
    obj['round2_keep_recipe']=tag;obj['round2_keep_component']='hall-front-oriel';obj['projection_component']='hall-front-oriel'
    return {'object':obj.name,'faces':len(faces),'nonmanifold_edges':invalid,'degenerate_faces':degenerate,
            'body_bottom':bottom,'body_top':top,'opening_heights':[opening_bottom,opening_top],'projecting_depth':20}


def refine_gallery_posts():
    """Straight structural posts measured in revealed artwork and native masks.

    The native silhouettes are conditional occluders. The first post's lower
    end is hidden by a foreground railing, so the physical foot continues to
    the known floor rather than ending at the mask's cropped silhouette.
    """
    working=bpy.data.collections['Derby Working']
    source=next(o for o in working.objects if o.type=='MESH' and not o.hide_render
                and o.get('source_node')=='building-223')
    tag='great-keep-hall-gallery-posts-v1'
    existing=[o for o in working.objects if o.get('round2_keep_recipe')==tag]
    if existing:
        if len(existing)!=3:raise ValueError('Partial gallery post recipe')
        for obj in existing:obj['projection_component']=obj['round2_keep_component']
        return {'status':'already-applied'}
    reports=[]
    tangent=Vector((129.43988,174.92786,0)).normalized()
    for number,x,mask in ((1,754.0,185),(2,779.0,183),(3,805.0,184)):
        y=-1945.85571+(x-676.50201)*174.92786/129.43988+2
        rings=[(219.75,4),(223,4),(229,2.5),(295,2.5),(301,3),(306.212,4.0)]
        vertices=[];faces=[]
        for z,radius in rings:
            for i in range(8):
                angle=2*math.pi*i/8
                vertices.append(Vector((x+radius*math.cos(angle),y+radius*math.sin(angle),z)))
        for j in range(len(rings)-1):
            for i in range(8):
                faces.append((j*8+i,j*8+(i+1)%8,(j+1)*8+(i+1)%8,(j+1)*8+i))
        faces.extend([tuple(reversed(range(8))),tuple(range((len(rings)-1)*8,len(rings)*8))])
        if mask==183:
            # The diagonal capital is visible in this particular native mask.
            a=Vector((x,y,294));b=Vector((x,y,306.212))+tangent*9
            direction=(b-a).normalized()
            side=Vector((-tangent.y,tangent.x,0))*1.3
            cross=direction.cross(side).normalized()*1.3
            base=len(vertices)
            vertices.extend(p+u*side+v*cross for p in (a,b) for u,v in ((-1,-1),(1,-1),(1,1),(-1,1)))
            faces.extend(tuple(base+i for i in f) for f in
                         ((3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)))
        mesh=bpy.data.meshes.new(f'Great Keep / gallery post {number:02}')
        mesh.from_pydata(vertices,[],faces)
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        invalid=sum(not e.is_manifold for e in bm.edges)
        degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
        bm.to_mesh(mesh);bm.free()
        if invalid or degenerate:raise ValueError('Invalid gallery post')
        neutral=bpy.data.materials.get('Great Keep / fireplace unknown')
        if neutral is None:
            neutral=bpy.data.materials.new('Great Keep / fireplace unknown')
            neutral.diffuse_color=(.25,.25,.25,1)
        mesh.materials.append(neutral)
        for original in source.data.uv_layers:
            uv=mesh.uv_layers.new(name=original.name)
            for loop in uv.data:loop.uv=(.5,.5)
        obj=bpy.data.objects.new(f'Great Keep / Hall gallery support post {number:02}',mesh)
        working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
        for key in source.keys():
            if not key.startswith('reprojection_'):obj[key]=source[key]
        obj['round2_keep_recipe']=tag
        obj['round2_keep_component']=f'hall-gallery-post-{number:02}'
        obj['projection_component']=obj['round2_keep_component']
        obj['reviewed_native_mask']=mask
        reports.append({'object':obj.name,'source_node':'building-223','mask_index':mask,
                        'mask_layer':2,'floor':219.75,'gallery_underside':306.212,
                        'faces':len(faces),'nonmanifold_edges':invalid,'degenerate_faces':degenerate})
    return {'posts':reports,'source_state':'patch-000 revealed'}


def refine_east_buttresses():
    """Model two visible east hall piers and their sloping tile copings.

    The roof-cap quadrilaterals were checked as projected overlays on covered
    artwork. Bases continue behind courtyard masonry; no hidden trim is added.
    """
    working=bpy.data.collections['Derby Working']
    tag='great-keep-hall-east-buttresses-v1'
    previous=[o for o in working.objects if o.get('round2_keep_recipe')==tag]
    if previous:
        if len(previous)!=2:raise ValueError('Partial east buttress recipe')
        for obj in previous:obj['projection_component']=obj['round2_keep_component']
        return {'status':'already-applied'}
    source=next(o for o in working.objects if o.type=='MESH' and not o.hide_render
                and o.get('source_node')=='building-179')
    normal=Vector((.833886,-.551937,0));tangent=Vector((.551937,.833886,0))
    reports=[]
    for number,x in enumerate((986.5,1040),1):
        y=-1938.089+(x-990)*(-normal.x/normal.y)
        center=Vector((x,y,0));vertices=[];faces=[]
        def prism(width,depth,lower,back_top,front_top):
            base=len(vertices)
            for top in (False,True):
                for lateral,forward in ((-1,-1),(1,-1),(1,depth),(-1,depth)):
                    p=center+tangent*(lateral*width/2)+normal*forward
                    p.z=(back_top if forward<0 else front_top) if top else lower
                    vertices.append(p)
            faces.extend(tuple(base+i for i in f) for f in
                         ((3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)))
        prism(20,16,0,430,400)
        # The projecting collar is visibly continuous across each pier.
        prism(24,19,231,239,239)
        mesh=bpy.data.meshes.new(f'Great Keep / east hall buttress {number:02}')
        mesh.from_pydata(vertices,[],faces)
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        invalid=sum(not e.is_manifold for e in bm.edges)
        degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
        bm.to_mesh(mesh);bm.free()
        if invalid or degenerate:raise ValueError('Invalid east hall buttress')
        neutral=bpy.data.materials.get('Great Keep / fireplace unknown')
        if neutral is None:
            neutral=bpy.data.materials.new('Great Keep / fireplace unknown')
            neutral.diffuse_color=(.25,.25,.25,1)
        mesh.materials.append(neutral)
        for original in source.data.uv_layers:
            uv=mesh.uv_layers.new(name=original.name)
            for loop in uv.data:loop.uv=(.5,.5)
        obj=bpy.data.objects.new(f'Great Keep / Hall east buttress {number:02}',mesh)
        working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
        for key in source.keys():
            if not key.startswith('reprojection_'):obj[key]=source[key]
        obj['round2_keep_recipe']=tag
        obj['round2_keep_component']=f'hall-east-buttress-{number:02}'
        obj['projection_component']=obj['round2_keep_component']
        reports.append({'object':obj.name,'source_node':'building-179','faces':len(faces),
                        'nonmanifold_edges':invalid,'degenerate_faces':degenerate,
                        'projecting_depth':16,'cap_back_height':430,'cap_front_height':400,
                        'width':20,'collar_height':[231,239]})
    return {'buttresses':reports,'source_state':'covered exterior',
            'inference':'Concealed depth and lower continuation use wall-normal structural extrusion'}
