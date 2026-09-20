"""Source112 is a horizontal hoop-bound cask, despite its legacy asset ID.

Keep derby-east-bailey-crate stable for saved maps. The reviewed render model
has a bowed timber body, inset end heads, and two raised iron hoops.
"""
import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Matrix,Vector

ASSET='derby-east-bailey-crate'
TAG='east-bailey-horizontal-cask-v1'


def _material(kind):
    name='East Bailey cask / '+kind
    material=bpy.data.materials.get(name)
    if material:return material
    material=bpy.data.materials.new(name);material.use_nodes=True
    nodes=material.node_tree.nodes;nodes.clear()
    output=nodes.new('ShaderNodeOutputMaterial');emission=nodes.new('ShaderNodeEmission')
    material.node_tree.links.new(emission.outputs[0],output.inputs[0])
    if kind=='iron':
        emission.inputs['Color'].default_value=(.032,.027,.017,1)
    else:
        path=Path(__file__).parent.parent/'work/derby-refinement/interior-layers/covered.png'
        source=bpy.data.images.load(str(path),check_existing=True)
        width,height=source.size;pixels=[]
        # The center of the observed timber head excludes the iron rim.
        x,y,size=1496,height-1336,6
        for row in range(y,y+size):pixels.extend(source.pixels[(row*width+x)*4:(row*width+x+size)*4])
        image=bpy.data.images.new(name,width=size,height=size,alpha=True);image.pixels=pixels;image.pack()
        texture=nodes.new('ShaderNodeTexImage');texture.image=image
        uv=nodes.new('ShaderNodeUVMap');uv.uv_map='UVMap'
        material.node_tree.links.new(uv.outputs[0],texture.inputs[0]);material.node_tree.links.new(texture.outputs[0],emission.inputs['Color'])
    return material


def _component(source,name,vertices,faces,material):
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    defects={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
             'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
    bm.to_mesh(mesh);bm.free()
    if any(defects.values()):raise ValueError(f'{name}: {defects}')
    mesh.materials.append(material);uv=mesh.uv_layers.new(name='UVMap');uv.active_render=True
    for face in mesh.polygons:
        axis=max(range(3),key=lambda i:abs(face.normal[i]));axes=[i for i in range(3) if i!=axis]
        for loop in face.loop_indices:
            p=mesh.vertices[mesh.loops[loop].vertex_index].co
            uv.data[loop].uv=(p[axes[0]]/6,p[axes[1]]/6)
    attr=mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    for value in attr.data:value.value=0
    obj=bpy.data.objects.new(name,mesh);bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith('reprojection_'):obj[key]=source[key]
    obj['asset_name']='East Bailey Barrel';obj['part_name']='Lying barrel'
    obj['east_bailey_cask_recipe']=TAG
    obj['projection_min_cosine']=.12
    return obj,defects


def refine():
    bpy.context.view_layer.update();working=bpy.data.collections['Derby Working']
    visible=[o for o in working.objects if o.type=='MESH' and not o.hide_render and o.get('source_node')=='building-112']
    if visible and all(o.get('east_bailey_cask_recipe')==TAG for o in visible):
        if len(visible)!=3:raise ValueError('Incomplete cask refinement')
        return {'asset':ASSET,'status':'already-refined'}
    if len(visible)!=1:raise ValueError('Expected one source112 volume')
    source=visible[0];points=[source.matrix_world@v.co for v in source.data.vertices]
    front=(points[16]+points[19])/2;back=(points[17]+points[18])/2
    bottom=min(p.z for p in points);height=max(p.z for p in points)-bottom
    front.z=back.z=bottom+height/2
    axis=(front-back).normalized();side=(points[16]-points[19]).normalized()
    radial_width=(points[16]-points[19]).length/2;radial_height=height/2
    count=32
    def position(t,factor,angle):
        return back.lerp(front,t)+side*(radial_width*factor*math.cos(angle))+Vector((0,0,radial_height*factor*math.sin(angle)))
    profile=[(.025,0),(.025,.89),(0,.94),(.06,.965),(.2,.99),(.5,1),
             (.8,.99),(.94,.965),(1,.94),(.975,.89),(.975,0)]
    # End centers are single vertices, avoiding collapsed zero-radius rings.
    vertices=[back.lerp(front,profile[0][0])]
    for t,radius in profile[1:-1]:
        vertices.extend(position(t,radius,2*math.pi*i/count) for i in range(count))
    vertices.append(back.lerp(front,profile[-1][0]));last=len(vertices)-1
    faces=[(0,1+(i+1)%count,1+i) for i in range(count)]
    for ring in range(len(profile)-3):
        a,b=1+ring*count,1+(ring+1)*count
        faces.extend((a+i,a+(i+1)%count,b+(i+1)%count,b+i) for i in range(count))
    end=1+(len(profile)-3)*count
    faces.extend((end+i,end+(i+1)%count,last) for i in range(count))
    body,defects=_component(source,'East Bailey Barrel / Bowed timber body and inset heads',vertices,faces,_material('timber'))
    results=[{'component':body.name,**defects}]
    for number,t in enumerate((.2,.8),1):
        half_width=.035
        vertices=[]
        for offset,radius in ((-half_width,.985),(half_width,.985),(half_width,1.025),(-half_width,1.025)):
            vertices.extend(position(t+offset,radius,2*math.pi*i/count) for i in range(count))
        faces=[]
        for ring in range(4):
            a,b=ring*count,((ring+1)%4)*count
            faces.extend((a+i,a+(i+1)%count,b+(i+1)%count,b+i) for i in range(count))
        hoop,defects=_component(source,f'East Bailey Barrel / Iron hoop {number:02}',vertices,faces,_material('iron'))
        hoop['authored_detail_material']='iron'
        results.append({'component':hoop.name,**defects})
    created=[o for o in working.objects if o.get('east_bailey_cask_recipe')==TAG]
    shift=bottom-min(v.co.z for o in created for v in o.data.vertices)
    for obj in created:
        for vertex in obj.data.vertices:vertex.co.z+=shift
        obj.data.update()
    source.hide_render=True;source.hide_set(True)
    return {'asset':ASSET,'identified_as':'horizontal barrel','components':results,
            'ground_z':min(v.co.z for o in created for v in o.data.vertices),'source_ground_z':bottom,
            'source_node':'building-112','preserved_parent':body.parent.name}


def finalize_materials():
    """After projection, retain the observed iron on newly raised hoop reveals.

The source is only a few dozen pixels across: changed ring surfaces can sample
adjacent timber. This explicit authored material is independent of UV visibility.
    """
    hoops=[o for o in bpy.data.collections['Derby Working'].objects
           if o.get('east_bailey_cask_recipe')==TAG and not o.hide_render
           and o.get('authored_detail_material')=='iron']
    if len(hoops)!=2:raise ValueError('Expected both authored iron hoops')
    for obj in hoops:
        index=next((i for i,m in enumerate(obj.data.materials) if m==_material('iron')),None)
        if index is None:raise ValueError('Missing authored iron material')
        for face in obj.data.polygons:face.material_index=index
    return {'authored_iron_hoops':len(hoops)}
