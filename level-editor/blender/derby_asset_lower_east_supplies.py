"""Replace the sloped supply obstacle with its painted plank cart and barrel."""
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG='lower_east_supply_cart_refinement'
UV='Cart fallback'


class Geometry:
    def __init__(self):
        self.vertices=[]
        self.faces=[]

    def add(self, vertices, faces):
        n=len(self.vertices)
        self.vertices.extend(vertices)
        self.faces.extend(tuple(n+i for i in face) for face in faces)

    def box(self, center, a, b, c):
        self.add([center+x*a+y*b+z*c for z in (-1,1) for y in (-1,1) for x in (-1,1)],
                 [(0,2,3,1),(4,5,7,6),(0,1,5,4),(2,6,7,3),(0,4,6,2),(1,3,7,5)])

    def ring(self, center, a, b, normal, outer, inner, thickness, count=24):
        vertices=[]
        for z,r in ((-1,outer),(1,outer),(-1,inner),(1,inner)):
            vertices += [center+normal*z*thickness/2+r*(a*math.cos(i*math.tau/count)+b*math.sin(i*math.tau/count)) for i in range(count)]
        faces=[]
        for i in range(count):
            j=(i+1)%count
            faces.extend(((i,j,j+count,i+count),(i+2*count,i+3*count,j+3*count,j+2*count),
                          (i,i+2*count,j+2*count,j),(i+count,j+count,j+3*count,i+3*count)))
        self.add(vertices,faces)


def _material():
    name='Derby / supply cart concealed wood'
    material=bpy.data.materials.get(name)
    if material:return material
    image=next((image for image in bpy.data.images if Path(image.filepath).name=='covered.png'),None)
    if image is None:raise ValueError('Load the covered Derby artwork before refining the cart')
    image.pack()
    material=bpy.data.materials.new(name);material.use_nodes=True
    nodes=material.node_tree.nodes;links=material.node_tree.links;nodes.clear()
    output=nodes.new('ShaderNodeOutputMaterial');emission=nodes.new('ShaderNodeEmission')
    texture=nodes.new('ShaderNodeTexImage');texture.image=image;texture.extension='CLIP'
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map=UV
    links.new(uv.outputs['UV'],texture.inputs['Vector'])
    links.new(texture.outputs['Color'],emission.inputs['Color']);links.new(emission.outputs[0],output.inputs['Surface'])
    return material


def _object(source, name, geometry, material, donor):
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(geometry.vertices,[],geometry.faces);mesh.update()
    mesh.materials.append(material)
    uv=mesh.uv_layers.new(name=UV)
    image=next(n.image for n in material.node_tree.nodes if n.type=='TEX_IMAGE')
    width,height=image.size
    for p in mesh.polygons:
        for li in p.loop_indices:
            v=mesh.vertices[mesh.loops[li].vertex_index].co-p.center
            # Concealed faces sample observed wood, not walls/stairs that happen
            # to cover them in the photograph. Visible faces are reprojected.
            x=donor[0]+max(-1.5,min(1.5,v.x*.12))
            y=donor[1]+max(-1.5,min(1.5,v.z*.12))
            uv.data[li].uv=(x/width,1-y/height)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    # Interpolate each plank's donor UV while splitting visibility cells, so
    # subdivision does not restart a texture patch on every tiny triangle.
    bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=5 if 'cargo body' in name else 2,
                             use_grid_fill=True,smooth=0)
    bmesh.ops.triangulate(bm,faces=list(bm.faces))
    defects={'nonmanifold':sum(not e.is_manifold for e in bm.edges),'degenerate':sum(f.calc_area()<1e-6 for f in bm.faces)}
    if any(defects.values()):bm.free();raise ValueError(f'Invalid cart geometry: {name}: {defects}')
    bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(name,mesh);bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():obj[key]=source[key]
    obj[TAG]=1;obj['asset_name']='Lower Bailey Supply Cart';obj['part_name']='Supply cart with barrel'
    return {'object':name,'triangles':len(mesh.polygons),'validation':defects}


def refine():
    collection=bpy.data.collections['Derby Working']
    previous=[o for o in collection.objects if o.get(TAG)]
    if previous:
        if len(previous)!=4 or any(o.get('source_node')!='building-075' for o in previous):
            raise ValueError('Incomplete supply cart refinement')
        return {'status':'existing','objects':[o.name for o in previous]}
    source=next(o for o in collection.objects if o.get('source_node')=='building-075' and not o.hide_render)
    bpy.context.view_layer.update()
    world=[source.matrix_world@v.co for v in source.data.vertices]
    back=(world[17]+world[18])/2;front=(world[16]+world[19])/2
    u=front-back;u.z=0;length=u.length;u.normalize()
    v=world[18]-world[17];v.z=0;width=v.length;v.normalize()
    center=(front+back)/2;up=Vector((0,0,1))

    def point(s,t,z_offset=0):
        p=back.lerp(front,s)+v*t
        p.z+=z_offset
        return p

    body=Geometry()
    # Retain the observed pitch of the wagon; board ends meet the same sloping
    # upper rim as its source obstacle. Three planks leave narrow real joints.
    pitched=(front-back).normalized()
    for i in range(6):
        body.box(point((i+.5)/6,0,-25),pitched*(front-back).length/12,v*(width/2-1),up*.8)
    for side in (-1,1):
        for row in range(3):
            body.box(point(.5,side*(width/2-1),-21+row*8),pitched*(front-back).length/2,v*.8,up*3.6)
        for s in (.06,.5,.94):
            body.box(point(s,side*(width/2+.1),-10),u*1.1,v*1.1,up*14)
    for end in (0,1):
        for row in range(3):
            body.box(point(end,0,-21+row*8),u*.8,v*(width/2-1),up*3.6)
    material=_material()
    report=[_object(source,'Lower Bailey Supply Cart / Plank cargo body',body,material,(980,1815))]
    axle=point(.39,0);axle.z=19
    for side,label in ((1,'Near'),(-1,'Far')):
        wheel=Geometry();wheel_center=axle+v*side*(width/2+2)
        wheel.ring(wheel_center,u,up,v,20,17,2.5)
        for i in range(10):
            axis=u*math.cos(i*math.tau/10)+up*math.sin(i*math.tau/10)
            cross=v.cross(axis).normalized()
            wheel.box(wheel_center+axis*9,axis*8,cross*.65,v*.8)
        wheel.box(wheel_center,u*2.2,up*2.2,v*2.1)
        report.append(_object(source,f'Lower Bailey Supply Cart / {label} spoked wheel',wheel,material,(1004,1837)))
    barrel=Geometry();barrel_center=Vector((992,-3198,0));count=20
    vertices=[]
    for z,r in ((23,8),(27,9.5),(36,10),(45,9.5),(49,8)):
        vertices += [barrel_center+Vector((r*math.cos(i*math.tau/count),r*math.sin(i*math.tau/count),z)) for i in range(count)]
    faces=[tuple(reversed(range(count))),tuple(range(4*count,5*count))]
    for ring in range(4):
        faces += [(ring*count+i,ring*count+(i+1)%count,(ring+1)*count+(i+1)%count,(ring+1)*count+i) for i in range(count)]
    barrel.add(vertices,faces)
    for z,r in ((28,9.7),(43,9.9)):
        barrel.ring(barrel_center+Vector((0,0,z)),Vector((1,0,0)),Vector((0,1,0)),up,r+.4,r-.2,1.5,count=20)
    report.append(_object(source,'Lower Bailey Supply Cart / Barrel cargo',barrel,material,(993,1801)))
    source.hide_render=True;source.hide_set(True)
    source.parent.name='Lower Bailey Supply Cart';source.parent['asset_name']='Lower Bailey Supply Cart'
    bpy.context.view_layer.update()
    return {'status':'created','objects':report,
            'remaining':'Far wheel is inferred by axle symmetry; concealed wood uses local donors. No tow shaft is added where obscured.'}
