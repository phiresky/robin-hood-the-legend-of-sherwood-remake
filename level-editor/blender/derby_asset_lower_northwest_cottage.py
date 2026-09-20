"""Audited northwest cottage roof, yard fence and hollow wooden vessels.

The painted hip ends, projecting eaves, fence openings and vessel rims replace
collision-volume silhouettes. Source collision records remain unchanged.
"""
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = 'derby-northwest-cottage-v1'
NAMES = {62:'Yard barrel', 63:'Yard fence', 64:'Wash tub',
         65:'North thatch roof and wall',66:'South thatch roof and wall'}


class Builder:
    def __init__(self, source):
        self.source=source
        self.points=[]
        self.faces=[]
        self.surfaces=[]
        self.donors=[]
        self.world=source.matrix_world
        self.vertices=[self.world@v.co for v in source.data.vertices]
        self.uv=next((u for u in source.data.uv_layers if u.active_render),source.data.uv_layers.active)
        fallback=source.data.attributes.get('reprojection_fallback_material')
        for face in source.data.polygons:
            coords=[self.vertices[i] for i in face.vertices]
            matrix=Matrix([self.project(p) for p in coords]).transposed()
            if len(coords)!=3 or abs(matrix.determinant())<1e-7:
                continue
            normal=(self.world.to_3x3().inverted().transposed()@face.normal).normalized()
            self.donors.append((normal,sum(coords,Vector())/3,matrix.inverted(),
                [self.uv.data[i].uv.copy() for i in face.loop_indices],
                fallback.data[face.index].value if fallback else face.material_index))

    @staticmethod
    def project(p):
        return Vector((p.x,-p.y*.573576436351046-p.z*.819152044288992,1))

    def face(self, points, surface=0):
        self.faces.append(tuple(range(len(self.points),len(self.points)+len(points))))
        self.points.extend(p.copy() for p in points)
        self.surfaces.append(surface)

    def box(self,a,b,width,bottom,top):
        tangent=(b-a).normalized()
        side=Vector((-tangent.y,tangent.x,0))*width/2
        ring=[a-side,b-side,b+side,a+side]
        low=[Vector((p.x,p.y,bottom)) for p in ring]
        high=[Vector((p.x,p.y,top)) for p in ring]
        self.face(low[::-1]);self.face(high)
        for i in range(4):self.face([low[i],low[(i+1)%4],high[(i+1)%4],high[i]])

    def finish(self,name):
        mesh=bpy.data.meshes.new(name)
        mesh.from_pydata(self.points,[],self.faces)
        mesh.update()
        surface=mesh.attributes.new('northwest_surface','INT','FACE')
        for item,value in zip(surface.data,self.surfaces):item.value=value
        orient=bmesh.new();orient.from_mesh(mesh)
        bmesh.ops.remove_doubles(orient,verts=list(orient.verts),dist=.0001)
        bmesh.ops.recalc_face_normals(orient,faces=list(orient.faces))
        orient.to_mesh(mesh);orient.free()
        for mat in self.source.data.materials:mesh.materials.append(mat)
        uv=mesh.uv_layers.new(name=self.uv.name)
        uv.active_render=True
        for face in mesh.polygons:
            normal=face.normal
            center=face.center
            donor=max(self.donors,key=lambda d:d[0].dot(normal)*100000-(d[1]-center).length_squared)
            _,_,inverse,values,material=donor
            face.material_index=material
            for i in face.loop_indices:
                weights=inverse@self.project(mesh.vertices[mesh.loops[i].vertex_index].co)
                uv.data[i].uv=sum((values[j]*weights[j] for j in range(3)),Vector((0,0)))
        if any(self.surfaces):
            # The north slope is almost edge-on in the source. Reusing its old
            # atlas stretches a single ridge row across the whole hidden slope.
            # Reflect the observed south thatch across the ridge for fallback.
            image_path=Path(__file__).resolve().parents[1]/'work/derby-refinement/interior-layers/covered.png'
            image=bpy.data.images.load(str(image_path),check_existing=True)
            image.pack()
            width,height=image.size
            layer=mesh.uv_layers.new(name='Northwest surface fallback')
            material=bpy.data.materials.new(name+' / sampled roof and timber fallback')
            material.use_nodes=True
            nodes=material.node_tree.nodes
            nodes.clear()
            texture=nodes.new('ShaderNodeTexImage');texture.image=image
            coordinates=nodes.new('ShaderNodeUVMap');coordinates.uv_map=layer.name
            shader=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial')
            material.node_tree.links.new(coordinates.outputs['UV'],texture.inputs['Vector'])
            material.node_tree.links.new(texture.outputs['Color'],shader.inputs['Color'])
            material.node_tree.links.new(shader.outputs[0],output.inputs['Surface'])
            mesh.materials.append(material)
            ridge=Vector((493.22,-3196.89,0))
            tangent=Vector((115.75,50.53,0)).normalized()
            across=Vector((-tangent.y,tangent.x,0))
            for face in mesh.polygons:
                surface=mesh.attributes['northwest_surface'].data[face.index].value
                if not surface or (surface==1 and face.normal.z < .1) or (surface==2 and abs(face.normal.z)>.1):
                    continue
                face.material_index=len(mesh.materials)-1
                for i in face.loop_indices:
                    point=mesh.vertices[mesh.loops[i].vertex_index].co.copy()
                    if surface==1:
                        distance=(point-ridge).dot(across)
                        if abs(face.normal.dot(tangent))>.2:
                            u=.28+.44*max(0,min(1,(distance+51)/123))
                            sample=ridge+tangent*(u*126.3)-across*(50.9*max(.05,min(.95,(121.1-point.z)/51.1)))
                            point=Vector((sample.x,sample.y,point.z))
                        elif distance>0:point-=across*distance*1.70
                    else:
                        # Castle occlusion leaves the west and rear walls without
                        # observed pixels. Reuse the observed timber facade rather
                        # than stretching the atlas's soil-colored edge padding.
                        u=((point-ridge).dot(across)+51)/123 if abs(face.normal.dot(tangent))>.7 else (point-ridge).dot(tangent)/126.3
                        sample=Vector((513.61,-3243.59,0)).lerp(Vector((629.36,-3193.,0)),max(.03,min(.97,u)))
                        point=Vector((sample.x,sample.y,point.z))
                    pixel=self.project(point)
                    layer.data[i].uv=(pixel.x/width,1-pixel.y/height)
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.0001)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bad_edges=sum(not e.is_manifold for e in bm.edges)
        bad_faces=sum(f.calc_area()<1e-7 for f in bm.faces)
        # Subcomponents such as rails overlap intentionally but each shell is closed.
        if bad_edges or bad_faces:
            bm.free();bpy.data.meshes.remove(mesh)
            raise ValueError(f'{name}: {bad_edges} nonmanifold edges; {bad_faces} degenerate faces')
        bmesh.ops.triangulate(bm,faces=list(bm.faces))
        bm.to_mesh(mesh);bm.free()
        obj=bpy.data.objects.new(name,mesh)
        bpy.data.collections['Derby Working'].objects.link(obj)
        obj.parent=self.source.parent
        obj.matrix_world=Matrix.Identity(4)
        for key in self.source.keys():obj[key]=self.source[key]
        obj['refinement_recipe']=TAG
        if any(self.surfaces):obj['projection_min_cosine']=.12
        obj['part_name']=NAMES[int(obj['source_node'].split('-')[-1])]
        obj['todo']='Concealed surfaces retain atlas fallback; refresh projection after all geometry edits.'
        self.source.hide_render=True
        self.source.hide_set(True)
        self.source['replaced_by']=obj.name
        bpy.context.view_layer.update()
        return {'source_node':obj['source_node'],'object':obj.name,'faces':len(mesh.polygons),
                'nonmanifold_edges':bad_edges,'degenerate_faces':bad_faces}


def vessel(source,node):
    build=Builder(source)
    count=24
    center=Vector((499.2,-3238.0,0)) if node==62 else Vector((449.7,-3273.4,0))
    rx,ry,height=(6.6,6.6,13.0) if node==62 else (16.,19.,11.2)
    # Barrel bulges between hoops; the broad tub flares toward its open mouth.
    specs=[(0,.82),(height*.48,1),(height,.87),(height,.70),(height*.2,.64)] if node==62 else [(0,.77),(height*.5,.89),(height,1),(height,.90),(2.,.69)]
    rings=[]
    for z,scale in specs:
        rings.append([center+Vector((rx*scale*math.cos(i*math.tau/count),ry*scale*math.sin(i*math.tau/count),z)) for i in range(count)])
    for low,high in zip(rings,rings[1:]):
        for i in range(count):build.face([low[i],low[(i+1)%count],high[(i+1)%count],high[i]])
    build.face(rings[0][::-1]);build.face(rings[-1])
    return build.finish(source['asset_name']+' / '+NAMES[node])


def fence(source):
    build=Builder(source)
    vertices=build.vertices
    a,b,c=[vertices[i].copy() for i in (25,24,29)]
    for p in (a,b,c):p.z=0
    # Post positions were read from the covered artwork along both yard edges.
    sections=[(a,b,[445.5,459,473,481.9]),(b,c,[481.9,505,531,558,581,597.5])]
    for start,end,post_x in sections:
        for z in (7.,20.):build.box(start,end,2.1,z,z+3.5)
        tangent=(end-start).normalized()
        for x in post_x:
            center=start.lerp(end,(x-start.x)/(end.x-start.x))
            build.box(center-tangent*1.7,center+tangent*1.7,3.4,0,29.25)
    return build.finish(source['asset_name']+' / Yard fence with open rails')


def roof(source,node):
    build=Builder(source)
    # Both halves share the same ridge and its lowered hip endpoints.
    ridge_a=Vector((493.22,-3196.89,121.10))
    ridge_b=Vector((608.97,-3146.36,121.10))
    high_a=ridge_a.lerp(ridge_b,.16)
    high_b=ridge_a.lerp(ridge_b,.89)
    if node==65:
        eave_a,eave_b=build.vertices[19].copy(),build.vertices[18].copy()
    else:
        eave_a,eave_b=build.vertices[18].copy(),build.vertices[19].copy()
    for p in (eave_a,eave_b):p.z=70.
    low_a,low_b=ridge_a.copy(),ridge_b.copy()
    low_a.z=low_b.z=70.
    # The top has three real slopes: the main thatch surface and two hip ends.
    build.face([eave_a,eave_b,high_b,high_a],1)
    build.face([eave_a,high_a,low_a],1)
    build.face([eave_b,low_b,high_b],1)
    build.face([low_a,high_a,high_b,low_b],1)
    boundary=[eave_a,eave_b,low_b,low_a]
    underside=[Vector((p.x,p.y,66.)) for p in boundary]
    build.face(underside[::-1])
    for i in range(4):build.face([boundary[i],boundary[(i+1)%4],underside[(i+1)%4],underside[i]])
    # Walls retreat under the painted thatch overhang; the roof outline stays fixed.
    center=Vector((546.5,-3161.5,0))
    upper=[Vector((p.x+(center.x-p.x)*.06,p.y+(center.y-p.y)*.06,66.)) for p in boundary]
    lower=[Vector((p.x,p.y,0)) for p in upper]
    build.face(upper);build.face(lower[::-1])
    for i in range(4):build.face([lower[i],lower[(i+1)%4],upper[(i+1)%4],upper[i]],2)
    return build.finish(source['asset_name']+' / '+NAMES[node]+' with hip ends')


def refine():
    bpy.context.view_layer.update()
    working=bpy.data.collections['Derby Working']
    result=[]
    for node in NAMES:
        matches=[o for o in working.all_objects if o.type=='MESH' and o.get('source_node')==f'building-{node:03d}' and not o.hide_render]
        if len(matches)!=1:raise ValueError(f'Expected one visible source {node}')
        source=matches[0]
        if source.get('refinement_recipe')==TAG:
            result.append({'source_node':source['source_node'],'status':'already-refined'})
        elif node in (62,64):result.append(vessel(source,node))
        elif node==63:result.append(fence(source))
        else:result.append(roof(source,node))
    return result
