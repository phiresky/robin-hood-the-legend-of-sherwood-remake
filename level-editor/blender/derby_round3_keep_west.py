"""West Tower rooftop stair cleanup and measured shallow doorway relief."""
import bpy
from mathutils import Vector

OWNED = {f'building-{n:03}' for n in [*range(130,143),144,145,239,240]}

def refine():
    objects = list(bpy.data.collections['Derby Working'].objects)
    wall = next(o for o in objects if o.type == 'MESH' and not o.hide_render
                and o.get('source_node') == 'building-136')
    if wall.get('west_door_relief_version') == 1:
        return {'status':'already-applied'}
    stair = next(o for o in objects if o.get('source_node') == 'building-134'
                 and 'modeled treads' in o.name)
    proxy = next(o for o in objects if o.get('source_node') == 'building-134'
                 and o != stair and o.type == 'MESH')
    proxy.hide_render = True
    proxy.hide_viewport = True
    proxy['west_retired_ramp'] = True
    # The existing closed stair spans the measured terrace and upper landing.
    # Its solid side and underside replace the obsolete full-height collision ramp.
    vertices = [wall.matrix_world @ v.co for v in wall.data.vertices]
    faces = [tuple(p.vertices) for p in wall.data.polygons]
    assert faces[2:4] == [(4,5,6),(4,7,5)], 'Unexpected turret front topology'
    # Retain the exact four outer corners; only the painted doorway is recessed.
    outer = [vertices[i] for i in (7,5,6,4)]
    def front(x,z):
        a,b=outer[:2]
        return Vector((x,a.y+(x-a.x)*(b.y-a.y)/(b.x-a.x),z))
    inner=[front(x,z) for x,z in ((304.8,543.25),(327.0,543.25),
                                 (327.0,589.0),(304.8,589.0))]
    normal=Vector((.0348995,-.9993908,0))
    start=len(vertices)
    vertices.extend(inner)
    vertices.extend(p-normal*4 for p in inner)
    faces=faces[:2]+faces[4:]
    corners=(7,5,6,4)
    for i in range(4):
        j=(i+1)%4
        faces.append((corners[i],corners[j],start+j,start+i))
        faces.append((start+i,start+j,start+4+j,start+4+i))
    faces.append(tuple(start+4+i for i in range(4)))
    mesh=bpy.data.meshes.new(wall.data.name+' / recessed rooftop door')
    inverse=wall.matrix_world.inverted()
    mesh.from_pydata([inverse@p for p in vertices],[],faces)
    for material in wall.data.materials:mesh.materials.append(material)
    for layer in wall.data.uv_layers:mesh.uv_layers.new(name=layer.name)
    mesh.update()
    wall.data=mesh
    wall['west_door_relief_version']=1
    return {'changed_nodes':['building-134','building-136'],
            'retired_proxy':proxy.name,'retained_stair':stair.name,
            'door_source_x':[304.8,327.0],'door_height':[543.25,589.0],
            'door_recess_depth':4,'door_depth_inferred':True,
            'door_is_closed_recess':True,'exterior_silhouette_unchanged':True}
