"""Courtyard drawbridge with distinct source artwork for each side and pose."""
import json
import math
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Matrix, Vector


def set_pose(obj, applied=False):
    amount = float(applied)
    if not 0 <= amount <= 1:
        raise ValueError('Drawbridge pose must be between zero and one')
    basis = Matrix(json.loads(obj['drawbridge_hinge_matrix']))
    obj.matrix_world = basis @ Matrix.Rotation(math.pi / 2 * amount, 4, 'X')
    obj['drawbridge_pose'] = amount
    obj['drawbridge_state'] = 'initial' if amount == 0 else 'applied' if amount == 1 else 'transition'
    for chain in bpy.data.collections['Derby Working'].objects:
        if chain.get('drawbridge_chain_for') != obj.name:
            continue
        x=float(chain['drawbridge_chain_x'])
        end=Vector((x,-111*math.sin(math.pi/2*amount),111*math.cos(math.pi/2*amount)))
        start=Vector((x,0,205))
        direction=(end-start).normalized();side=Vector((1,0,0));other=direction.cross(side).normalized()
        for vertex in chain.data.vertices:
            ring=vertex.index//8;t=ring/40
            angle=vertex.index%8*math.tau/8
            vertex.co=basis@(start.lerp(end,t)+.65*(math.cos(angle)*side+math.sin(angle)*other))
        chain['drawbridge_state']=obj['drawbridge_state']


def refine(manifest_path, applied=False):
    manifest_path = Path(manifest_path)
    manifest = json.loads(manifest_path.read_text())
    patch = next(p for p in manifest['mission_patches']
                 if p['mission'] == 'H03_Der_MK' and p['name'] == 'Derby - Pont_levis01')
    collection = bpy.data.collections['Derby Working']
    previous = [o for o in collection.objects if o.type == 'MESH' and o.get('source_node') == 'building-267']
    existing = next((o for o in previous if o.get('drawbridge_revision')), None)
    if existing:
        set_pose(existing, applied)
        return {'object': existing.name, 'already_applied': True, 'state': existing['drawbridge_state']}
    source = previous[0]
    sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
    # Hinge endpoints and deck tip match both endpoint sprite silhouettes.
    left, right = Vector((627, -1589 / sin, 0)), Vector((704, -1584 / sin, 0))
    axis = (right-left).normalized()
    basis = Matrix((axis, Vector((-axis.y, axis.x, 0)), Vector((0,0,1)))).transposed().to_4x4()
    basis.translation = left
    width, length, thickness = (right-left).length, 111, 3
    verts, faces = [], []
    # Small physical seams between planks, with real edge thickness.
    for step in range(12):
        lo, hi = length*step/12, length*(step+1)/12-.15
        offset = len(verts)
        verts.extend((x,y,z) for z in (lo,hi) for y in (-thickness/2,thickness/2) for x in (0,width))
        faces.extend(tuple(offset+i for i in face) for face in
                     ((0,1,5,4),(2,6,7,3),(0,4,6,2),(1,3,7,5),(0,2,3,1),(4,5,7,6)))
    mesh = bpy.data.meshes.new('Courtyard drawbridge / twelve timber planks')
    mesh.from_pydata(verts, [], faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(source['asset_name']+' / Hinged courtyard drawbridge',mesh)
    collection.objects.link(obj);obj.parent=source.parent
    for key in ('source_node','source_obstacle','asset_group','asset_name'):
        obj[key]=source[key]
    obj['part_name']='Hinged courtyard drawbridge'
    obj['drawbridge_revision']=1
    obj['drawbridge_hinge_matrix']=json.dumps([list(r) for r in basis])
    obj['mission_patch_profile']=patch['name']
    obj['mission_patch_mission']=patch['mission']
    obj['mission_patch_ids']=json.dumps([p['id'] for p in manifest['mission_patches']
                                      if p['name'] in ('Derby - Pont_levis01','Derby - Pont_levis01_mecanisme')])
    obj['mission_patch_state_json']=json.dumps(patch['state'])
    obj['drawbridge_pose_angles_degrees']=[0,90]
    obj['drawbridge_sight_note']='Raised state blocks sight; applied state retains the lowered deck mesh.'
    obj['projection_layer']='exterior'
    uv=mesh.uv_layers.new(name='Drawbridge state artwork')
    images=[]
    for state in ('initial','applied'):
        graphic=patch[state+'_graphic'];reference=bpy.data.images.load(str(manifest_path.parent/graphic['image']),check_existing=True);images.append(graphic)
        # Keyed transparency carries bright green RGB. Pack an opaque material
        # copy with a timber edge color, retaining the untouched source PNG.
        pixels=np.empty(len(reference.pixels),dtype=np.float32);reference.pixels.foreach_get(pixels);pixels=pixels.reshape((-1,4))
        pixels[:,:3]=pixels[:,:3]*pixels[:,3:4]+np.array((.10,.07,.04))*(1-pixels[:,3:4]);pixels[:,3]=1
        image=bpy.data.images.new('Drawbridge '+state+' opaque edge',width=reference.size[0],height=reference.size[1],alpha=False)
        image.pixels.foreach_set(pixels.ravel());image.pack()
        material=bpy.data.materials.new('Courtyard drawbridge / '+state+' artwork');material.use_nodes=True;material['projection_preserve']=True
        nodes=material.node_tree.nodes;nodes.clear();output=nodes.new('ShaderNodeOutputMaterial');emission=nodes.new('ShaderNodeEmission');texture=nodes.new('ShaderNodeTexImage');texture.image=image
        coord=nodes.new('ShaderNodeUVMap');coord.uv_map=uv.name
        material.node_tree.links.new(coord.outputs[0],texture.inputs[0]);material.node_tree.links.new(texture.outputs[0],emission.inputs[0]);material.node_tree.links.new(emission.outputs[0],output.inputs[0]);mesh.materials.append(material)
    edge=bpy.data.materials.new('Courtyard drawbridge / timber edge');edge.diffuse_color=(.12,.085,.05,1);edge['projection_preserve']=True;mesh.materials.append(edge)
    fallback=mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    for face in mesh.polygons:
        index=0 if face.normal.y < -.9 else 1 if face.normal.y > .9 else 2
        face.material_index=index;fallback.data[face.index].value=index
        graphic=images[min(index,1)];x,y,w,h=graphic['bbox']
        transform=basis if index==0 else basis @ Matrix.Rotation(math.pi/2,4,'X')
        for li in face.loop_indices:
            p=transform @ mesh.vertices[mesh.loops[li].vertex_index].co
            uv.data[li].uv=((p.x-x)/w,1-(-p.y*sin-p.z*cos-y)/h)
    for old in previous:
        old.hide_render=True;old.hide_set(True);old['replaced_by']=obj.name
    def accessory(name, vertices, polygons, material):
        data=bpy.data.meshes.new(name);data.from_pydata(vertices,[],polygons);data.materials.append(material)
        data.uv_layers.new(name='Accessory fallback');data.attributes.new('reprojection_fallback_material','INT','FACE')
        part=bpy.data.objects.new(source['asset_name']+' / '+name,data);collection.objects.link(part);part.parent=source.parent;part.matrix_world=Matrix.Identity(4)
        for key in ('source_node','source_obstacle','asset_group','asset_name'):part[key]=source[key]
        part['part_name']=name;part['projection_layer']='exterior';part['drawbridge_accessory']=True
        return part
    metal=bpy.data.materials.new('Drawbridge chain iron');metal.diffuse_color=(.07,.055,.04,1);metal['projection_preserve']=True
    for x in (1,width-1):
        ring_faces=[(r*8+k,r*8+(k+1)%8,(r+1)*8+(k+1)%8,(r+1)*8+k) for r in range(40) for k in range(8)]
        ring_faces.extend((tuple(reversed(range(8))),tuple(40*8+k for k in range(8))))
        chain=accessory('Suspension chain '+str(round(x)),[(0,0,0)]*(41*8),ring_faces,metal)
        chain['drawbridge_chain_for']=obj.name;chain['drawbridge_chain_x']=x
    # The original closure hid the portal floor. Keep actual floor depth when
    # the leaf lowers, instead of revealing unrelated background behind it.
    floor_material=bpy.data.materials.new('Drawbridge passage floor / reference');floor_material.use_nodes=True;floor_material['projection_preserve']=True
    nodes=floor_material.node_tree.nodes;nodes.clear();output=nodes.new('ShaderNodeOutputMaterial');emission=nodes.new('ShaderNodeEmission');texture=nodes.new('ShaderNodeTexImage')
    texture.image=bpy.data.images.load(str(manifest_path.parent/'covered.png'),check_existing=True);texture.image.pack()
    coord=nodes.new('ShaderNodeUVMap');coord.uv_map='Passage map projection'
    floor_material.node_tree.links.new(coord.outputs[0],texture.inputs[0]);floor_material.node_tree.links.new(texture.outputs[0],emission.inputs[0]);floor_material.node_tree.links.new(emission.outputs[0],output.inputs[0])
    floor=accessory('Drawbridge passage floor',[basis@Vector(v) for v in ((-1,0,.5),(width+1,0,.5),(width+30,205,.5),(-25,205,.5))],[(0,1,2,3)],floor_material)
    floor_uv=floor.data.uv_layers.new(name='Passage map projection');image_width,image_height=texture.image.size
    for loop in floor.data.loops:
        p=floor.data.vertices[loop.vertex_index].co;floor_uv.data[loop.index].uv=(p.x/image_width,1-(-p.y*sin-p.z*cos)/image_height)
    set_pose(obj,applied)
    return {'object':obj.name,'source_node':'building-267','planks':12,'state':obj['drawbridge_state'],
            'hinge_endpoints':[list(left),list(right)],'deck_length':length,
            'limitation':'Suspension chains use continuous iron strands; individual links and winch hardware remain unmodeled.'}
