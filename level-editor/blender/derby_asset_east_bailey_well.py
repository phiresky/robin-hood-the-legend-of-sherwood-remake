"""Covered well: supported tile canopy, open masonry shaft and yard bucket.

Each replacement keeps its original obstacle ID, logical parent and collision
metadata. The small bucket is a render component of the existing basin asset.
"""
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG='derby-east-bailey-covered-well-v1'


class MeshBuilder:
    def __init__(self, source):
        self.source=source
        self.vertices=[source.matrix_world@v.co for v in source.data.vertices]
        self.points=[];self.faces=[];self.kinds=[]
        self.layer=next((u for u in source.data.uv_layers if u.active_render),source.data.uv_layers.active)
        fallback=source.data.attributes.get('reprojection_fallback_material')
        self.donors=[]
        for face in source.data.polygons:
            points=[self.vertices[i] for i in face.vertices]
            if len(points)!=3:continue
            basis=Matrix([self.project(p) for p in points]).transposed()
            if abs(basis.determinant())<1e-7:continue
            normal=(source.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized()
            self.donors.append((normal,sum(points,Vector())/3,basis.inverted(),
                [self.layer.data[i].uv.copy() for i in face.loop_indices],
                fallback.data[face.index].value if fallback else face.material_index))

    @staticmethod
    def project(p):return Vector((p.x,-p.y*.573576436351046-p.z*.819152044288992,1))

    def face(self,points,kind=0):
        self.faces.append(tuple(range(len(self.points),len(self.points)+len(points))))
        self.points.extend(p.copy() for p in points)
        self.kinds.append(kind)

    def prism(self,ring,offset,kind=0):
        other=[p+offset for p in ring]
        self.face(ring,kind);self.face(other[::-1],kind)
        for i in range(len(ring)):
            self.face([ring[i],ring[(i+1)%len(ring)],other[(i+1)%len(ring)],other[i]],kind)

    def post(self,point):
        size=1.25
        ring=[Vector((point.x+x,point.y+y,0)) for x,y in [(-size,-size),(size,-size),(size,size),(-size,size)]]
        self.prism(ring,Vector((0,0,55)),1)

    def finish(self,label,minimum_cosine=.08):
        mesh=bpy.data.meshes.new(label)
        mesh.from_pydata(self.points,[],self.faces)
        attr=mesh.attributes.new('well_surface_kind','INT','FACE')
        for item,value in zip(attr.data,self.kinds):item.value=value
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.0001)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bad_edges=sum(not edge.is_manifold for edge in bm.edges)
        bad_faces=sum(face.calc_area()<1e-7 for face in bm.faces)
        if bad_edges or bad_faces:
            bm.free();bpy.data.meshes.remove(mesh)
            raise ValueError(f'{label}: {bad_edges} nonmanifold edges, {bad_faces} degenerate faces')
        bm.to_mesh(mesh);bm.free()
        for material in self.source.data.materials:mesh.materials.append(material)
        uv=mesh.uv_layers.new(name=self.layer.name);uv.active_render=True
        for face in mesh.polygons:
            donor=max(self.donors,key=lambda d: d[0].dot(face.normal)*1e5-(d[1]-face.center).length_squared)
            _,_,inverse,values,material=donor
            face.material_index=material
            for i in face.loop_indices:
                weights=inverse@self.project(mesh.vertices[mesh.loops[i].vertex_index].co)
                uv.data[i].uv=sum((values[j]*weights[j] for j in range(3)),Vector((0,0)))
        # Small source donors provide wood/interior fallback for newly exposed
        # surfaces. Visible faces subsequently receive visibility-aware projection.
        image_path=Path(__file__).resolve().parents[1]/'work/derby-refinement/interior-layers/covered.png'
        image=bpy.data.images.load(str(image_path),check_existing=True);image.pack()
        width,height=image.size
        sampled=mesh.uv_layers.new(name='Covered well donor')
        mat=bpy.data.materials.new(label+' / concealed donor')
        mat.use_nodes=True
        nodes=mat.node_tree.nodes;nodes.clear()
        coordinates=nodes.new('ShaderNodeUVMap');coordinates.uv_map=sampled.name
        texture=nodes.new('ShaderNodeTexImage');texture.image=image
        emission=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial')
        mat.node_tree.links.new(coordinates.outputs['UV'],texture.inputs['Vector'])
        mat.node_tree.links.new(texture.outputs['Color'],emission.inputs['Color'])
        mat.node_tree.links.new(emission.outputs[0],output.inputs['Surface'])
        mesh.materials.append(mat)
        donors={1:(1467.,1489.),2:(1447.,1468.),3:(1418.,1497.),4:(1449.,1490.),5:(0,0)}
        for face in mesh.polygons:
            kind=mesh.attributes['well_surface_kind'].data[face.index].value
            if kind not in donors:continue
            face.material_index=len(mesh.materials)-1
            x,y=donors[kind]
            for i in face.loop_indices:
                point=mesh.vertices[mesh.loops[i].vertex_index].co
                if kind==2:
                    pixel_x=1447+(point.x-1455.3)*.35
                    pixel_y=1468+(23.6-point.z)*.18
                elif kind==4:
                    # Reuse masonry away from the painted front support. Direct
                    # projection of partly hidden faces would bake that support
                    # into the shaft and duplicate it when viewed from the side.
                    angle=math.atan2(point.y+2595.5,point.x-1455.3)
                    # Unwrap around the cylinder instead of flattening side-on
                    # faces into one source column. Repeat a clean masonry patch.
                    center_angle=math.atan2(face.center.y+2595.5,face.center.x-1455.3)
                    angle=center_angle+math.atan2(math.sin(angle-center_angle),math.cos(angle-center_angle))
                    origin=math.floor(center_angle/(math.pi/2))*(math.pi/2)
                    pixel_x=1440+(angle-origin)/(math.pi/2)*20
                    pixel_y=1497-max(0,min(1,point.z/23.6))*13
                elif kind==5:
                    # Rear coping is hidden by tiles in the source; sample the
                    # observed front coping instead of baking roof into stone.
                    radius=math.hypot(point.x-1455.3,point.y+2595.5)
                    pixel_x=1452+(point.x-1455.3)/26.3*8
                    sample_y=-2595.5-math.sqrt(max(0,radius**2-(pixel_x-1455.3)**2))
                    pixel_y=-sample_y*.573576436351046-point.z*.819152044288992
                else:
                    pixel_x=x+(point.x-face.center.x)*.05
                    pixel_y=y+(point.z-face.center.z)*.12
                sampled.data[i].uv=(pixel_x/width,1-pixel_y/height)
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
        obj=bpy.data.objects.new(label,mesh)
        bpy.data.collections['Derby Working'].objects.link(obj)
        obj.parent=self.source.parent;obj.matrix_world=Matrix.Identity(4)
        for key in self.source.keys():obj[key]=self.source[key]
        obj['refinement_recipe']=TAG
        obj['projection_min_cosine']=minimum_cosine
        obj['todo']='Hidden wood and inner shaft use bounded source donor textures.'
        self.source.hide_render=True;self.source.hide_set(True)
        self.source['replaced_by']=obj.name
        bpy.context.view_layer.update()
        return {'source_node':obj['source_node'],'object':obj.name,'faces':len(mesh.polygons),
                'nonmanifold_edges':bad_edges,'degenerate_faces':bad_faces}


def canopy(source,node):
    build=MeshBuilder(source)
    top=[build.vertices[i].copy() for i in (12,13,14,15)]
    build.prism(top,Vector((0,0,-4)))
    # The source has short boarded gable ends, distinct from the pitched slab.
    if node==109:
        ridge_a,ridge_b=top[2],top[1]
        eave_a,eave_b=top[3],top[0]
    else:
        ridge_a,ridge_b=top[0],top[1]
        eave_a,eave_b=top[3],top[2]
    direction=(ridge_b-ridge_a).normalized()
    for ridge,eave,offset in ((ridge_a,eave_a,direction*1.5),(ridge_b,eave_b,-direction*1.5)):
        ring=[ridge-Vector((0,0,4.02)),eave-Vector((0,0,4.02)),Vector((eave.x,eave.y,52)),Vector((ridge.x,ridge.y,52))]
        build.prism(ring,offset)
        build.post(eave)
    return build.finish(source['asset_name']+(' / East tiles and timber supports' if node==109 else ' / West tiles and timber supports'))


def ring(build,center,rx,ry,levels,kinds,count=32):
    rings=[[center+Vector((rx*scale*math.cos(i*math.tau/count),ry*scale*math.sin(i*math.tau/count),z)) for i in range(count)] for z,scale in levels]
    for j,(a,b) in enumerate(zip(rings,rings[1:])):
        for i in range(count):build.face([a[i],a[(i+1)%count],b[(i+1)%count],b[i]],kinds[j])
    build.face(rings[0][::-1],kinds[0]);build.face(rings[-1],kinds[-1])


def basin(source):
    build=MeshBuilder(source)
    ring(build,Vector((1455.3,-2595.5,0)),26.3,26.5,
         [(0,.97),(21.5,1),(23.6,1.015),(23.6,.76),(2.,.76)], [4,4,5,2])
    # The circular wall has little trustworthy source area around the painted
    # posts. Restrict its direct projection; retain the clean masonry unwrap.
    report=build.finish(source['asset_name']+' / Open stone shaft',minimum_cosine=.8)
    bucket=MeshBuilder(source)
    ring(bucket,Vector((1418.,-2620.5,0)),5.4,5.4,
         [(0,.72),(9.,1),(9.,.78),(1.5,.62)], [3,3,3],24)
    report['bucket']=bucket.finish(source['asset_name']+' / Water bucket')
    report['ground_cleanup_pixels']=[1411,1486,1426,1507]
    return report


def refine():
    bpy.context.view_layer.update()
    working=bpy.data.collections['Derby Working']
    report=[]
    for node in (109,110,111):
        sources=[o for o in working.all_objects if o.type=='MESH' and o.get('source_node')==f'building-{node:03d}' and not o.hide_render]
        if sources and all(o.get('refinement_recipe')==TAG for o in sources):
            report.append({'source_node':f'building-{node:03d}','status':'already-refined','components':len(sources)})
            continue
        if len(sources)!=1:raise ValueError(f'Expected one visible source {node}')
        source=sources[0]
        if source.get('refinement_recipe')==TAG:
            report.append({'source_node':source['source_node'],'status':'already-refined'})
        elif node==111:report.append(basin(source))
        else:report.append(canopy(source,node))
    return report
