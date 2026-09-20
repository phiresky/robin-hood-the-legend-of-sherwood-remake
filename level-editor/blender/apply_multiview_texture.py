"""Project generated multiview artwork onto source-hidden asset faces."""
import hashlib
import json
import math
from pathlib import Path
import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree


def apply(manifest_path, image_path, report_path):
    """Apply generated artwork using the source-protected texture atlas."""
    return apply_baked(manifest_path, image_path, report_path)


def apply_baked(manifest_path, image_path, report_path, texels_per_unit=2, rear_bias=0.4, refresh_source=False):
    """Bake hidden texels into face islands while retaining original visible art.

    Unlike whole-face replacement, a polygon may contain both protected source
    pixels and generated pixels. No vertex or polygon positions are changed.
    """
    import numpy as np
    manifest=json.loads(Path(manifest_path).read_text())
    objects=[o for o in bpy.data.collections['Derby Working'].objects
             if o.type=='MESH' and not o.hide_render and o.get('asset_group')==manifest['asset_id']]
    if not objects:raise ValueError('No visible asset geometry')
    if texels_per_unit<=0:raise ValueError('Texture density must be positive')
    if any(o.data.uv_layers.get('Baked hidden artwork') or any(m and m.get('generated_source_sha256') for m in o.data.materials) for o in objects):
        raise ValueError('Restore the pre-application checkpoint before reapplying')
    image_hash=hashlib.sha256(Path(image_path).read_bytes()).hexdigest()
    bpy.context.view_layer.update()
    vertices=[];triangles=[]
    for obj in objects:
        offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        obj.data.calc_loop_triangles()
        triangles.extend(tuple(offset+i for i in t.vertices) for t in obj.data.loop_triangles)
    tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True)
    source=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))))
    generated=bpy.data.images.load(str(image_path),check_existing=False)
    gw,gh=generated.size
    if [gw,gh]!=[manifest['layout']['width'],manifest['layout']['height']]:
        raise ValueError('Generated image dimensions differ from camera manifest')
    arrays={}
    def pixels(image):
        if image.name not in arrays:
            data=np.empty(len(image.pixels),dtype=np.float32);image.pixels.foreach_get(data)
            arrays[image.name]=data.reshape(image.size[1],image.size[0],4)
        return arrays[image.name]
    generated_pixels=pixels(generated)
    source_pixels=pixels(bpy.data.images.load(manifest['source_image'],check_existing=False)) if refresh_source else None
    cameras=[]
    for view in manifest['views'][1:]:
        matrix=Matrix(view['camera_matrix_world'])
        cameras.append((view,matrix.inverted(),matrix.to_3x3()@Vector((0,0,1))))
    islands=[];cursor_x=2;cursor_y=2;row_height=0;atlas_width=4096
    for obj in objects:
        mesh=obj.data
        world=[obj.matrix_world@v.co for v in mesh.vertices]
        for face in mesh.polygons:
            points=[world[i] for i in face.vertices]
            center=sum(points,Vector())/len(points)
            normal=(obj.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized()
            # A whole face visible from the source needs no new sampling.
            checks=[center]+[p.lerp(center,.05) for p in points]
            source_reliable=normal.dot(source)>float(obj.get('projection_min_cosine',0.05))
            if not refresh_source and source_reliable and all(tree.ray_cast(p+source*.15,source)[0] is None for p in checks):continue
            candidates=[(normal.dot(direction)-rear_bias*abs(view['index']-4),view,inverse,direction)
                        for view,inverse,direction in cameras if normal.dot(direction)>.2]
            if not candidates:continue
            _,view,inverse,direction=max(candidates,key=lambda v:v[0])
            material=mesh.materials[face.material_index]
            if material is None or not material.use_nodes:continue
            texture=next((n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image),None)
            if texture is None:continue
            uvnode=next((n for n in material.node_tree.nodes if n.type=='UVMAP'),None)
            olduv=mesh.uv_layers.get(uvnode.uv_map) if uvnode else next((u for u in mesh.uv_layers if u.active_render),mesh.uv_layers.active)
            if olduv is None:continue
            origin=points[0];axis=max((p-origin for p in points),key=lambda p:p.length).normalized();vertical=normal.cross(axis).normalized()
            coords=[Vector(((p-origin).dot(axis),(p-origin).dot(vertical))) for p in points]
            low=Vector((min(p.x for p in coords),min(p.y for p in coords)))
            high=Vector((max(p.x for p in coords),max(p.y for p in coords)))
            size=high-low
            if min(size)<.01:continue
            w=min(1024,max(4,math.ceil(size.x*texels_per_unit)));h=min(1024,max(4,math.ceil(size.y*texels_per_unit)))
            if cursor_x+w+4>atlas_width:cursor_x=2;cursor_y+=row_height+4;row_height=0
            islands.append((obj,face.index,origin,axis,vertical,low,size,w,h,cursor_x,cursor_y,view,inverse,direction,olduv.name,texture.image,source_reliable))
            cursor_x+=w+4;row_height=max(row_height,h)
    atlas_height=cursor_y+row_height+2
    atlas=np.zeros((atlas_height,atlas_width,4),dtype=np.float32);atlas[:,:,3]=1
    changed=0;protected=0;faces=[]
    for obj,face_id,origin,axis,vertical,low,size,w,h,left,bottom,view,inverse,direction,old_name,old_image,source_reliable in islands:
        mesh=obj.data;face=mesh.polygons[face_id];olduv=mesh.uv_layers[old_name];oldpixels=pixels(old_image)
        face_triangles=[t for t in mesh.loop_triangles if t.polygon_index==face_id]
        ts=[]
        for triangle in face_triangles:
            p=[obj.matrix_world@mesh.vertices[i].co for i in triangle.vertices]
            q=[Vector(((v-origin).dot(axis),(v-origin).dot(vertical))) for v in p]
            a,b,c=q;det=(b.y-c.y)*(a.x-c.x)+(c.x-b.x)*(a.y-c.y)
            if abs(det)<1e-8:continue
            ts.append((p,q,det,[olduv.data[i].uv.copy() for i in triangle.loops]))
        yy,xx=np.mgrid[-2:h+2,-2:w+2]
        qx=low.x+(xx.ravel()+.5)*size.x/w;qy=low.y+(yy.ravel()+.5)*size.y/h
        count=len(qx);best=np.full(count,-np.inf);points=np.zeros((count,3));oldcoords=np.zeros((count,2))
        for p,coords,det,uvs in ts:
            a,b,c=coords
            wa=((b.y-c.y)*(qx-c.x)+(c.x-b.x)*(qy-c.y))/det
            wb=((c.y-a.y)*(qx-c.x)+(a.x-c.x)*(qy-c.y))/det
            weights=np.stack((wa,wb,1-wa-wb),axis=1);margin=weights.min(axis=1);take=margin>best
            points[take]=weights[take]@np.asarray(p);oldcoords[take]=weights[take]@np.asarray(uvs);best[take]=margin[take]
        def sample_many(data,coords):
            ih,iw=data.shape[:2]
            x=np.clip(coords[:,0]*iw-.5,0,iw-1);y=np.clip(coords[:,1]*ih-.5,0,ih-1)
            ix=x.astype(int);iy=y.astype(int);fx=(x-ix)[:,None];fy=(y-iy)[:,None]
            return (data[iy,ix]*(1-fx)+data[iy,np.minimum(ix+1,iw-1)]*fx)*(1-fy)+(data[np.minimum(iy+1,ih-1),ix]*(1-fx)+data[np.minimum(iy+1,ih-1),np.minimum(ix+1,iw-1)]*fx)*fy
        colors=sample_many(oldpixels,oldcoords)
        local=points@np.asarray(inverse.to_3x3()).T+np.asarray(inverse.translation)
        crop=view['crop'];scale=view['ortho_scale']
        generated_coords=np.stack(((crop['left']+(.5+local[:,0]/(scale*crop['width']/crop['height']))*crop['width'])/gw,
                                   (gh-crop['top']-(.5-local[:,1]/scale)*crop['height'])/gh),axis=1)
        generated_colors=sample_many(generated_pixels,generated_coords)
        eligible=generated_colors[:,:3].max(axis=1)>.035
        source_offset=source*.25;target_offset=direction*.25
        source_visible=np.zeros(count,dtype=bool)
        if refresh_source and source_reliable:
            for i in range(count):source_visible[i]=tree.ray_cast(Vector(points[i])+source_offset,source)[0] is None
            sh,sw=source_pixels.shape[:2]
            source_coords=np.stack((points[:,0]/sw,1-(-points[:,1]*math.sin(math.radians(35))-points[:,2]*math.cos(math.radians(35)))/sh),axis=1)
            colors[source_visible]=sample_many(source_pixels,source_coords[source_visible])
        for i in np.flatnonzero(eligible):
            point=Vector(points[i])
            eligible[i]=(not source_reliable or tree.ray_cast(point+source_offset,source)[0] is not None) and tree.ray_cast(point+target_offset,direction)[0] is None
        colors[eligible]=generated_colors[eligible]
        modified=int(eligible.sum());protected+=count-modified
        atlas[bottom-2:bottom+h+2,left-2:left+w+2]=colors.reshape(h+4,w+4,4)
        if modified or source_visible.any():
            faces.append((obj,face_id,origin,axis,vertical,low,size,w,h,left,bottom))
            changed+=modified
    image=bpy.data.images.new(manifest['asset_id']+' / source protected generated atlas',width=atlas_width,height=atlas_height,alpha=True)
    image.pixels.foreach_set(atlas.ravel());image.update();image.pack()
    uv_name='Baked hidden artwork'
    material=bpy.data.materials.new(manifest['asset_id']+' / baked hidden artwork');material.use_nodes=True;material['projection_preserve']=True
    material['generated_source_sha256']=image_hash
    material['generated_camera_manifest']=str(Path(manifest_path).resolve())
    nodes=material.node_tree.nodes;nodes.clear();links=material.node_tree.links
    uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv_name
    tex=nodes.new('ShaderNodeTexImage');tex.image=image
    emission=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial')
    links.new(uvnode.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],emission.inputs['Color']);links.new(emission.outputs[0],output.inputs['Surface'])
    slots={}
    for obj,face_id,origin,axis,vertical,low,size,w,h,left,bottom in faces:
        mesh=obj.data
        if obj not in slots:
            slots[obj]=len(mesh.materials);mesh.materials.append(material);mesh.uv_layers.new(name=uv_name)
        face=mesh.polygons[face_id];face.material_index=slots[obj]
        fallback=mesh.attributes.get('reprojection_fallback_material')
        if fallback:fallback.data[face_id].value=slots[obj]
        for loop_id in face.loop_indices:
            point=obj.matrix_world@mesh.vertices[mesh.loops[loop_id].vertex_index].co-origin
            q=Vector((point.dot(axis),point.dot(vertical)))-low
            mesh.uv_layers[uv_name].data[loop_id].uv=((left+q.x/size.x*w)/atlas_width,(bottom+q.y/size.y*h)/atlas_height)
    report={'changed_faces':len(faces),'generated_texels':changed,'protected_texels':protected,'atlas_size':[atlas_width,atlas_height],'geometry_changed':False,
            'image_sha256':image_hash,'camera_manifest':str(Path(manifest_path).resolve()),'texels_per_unit':texels_per_unit,'rear_bias':rear_bias,'refresh_source':refresh_source,
            'source_preservation':('Reliable source-facing texels refreshed from original artwork; grazing surfaces completed from generated views.' if refresh_source else 'Original visible colors resampled into partially hidden face islands; fully visible faces unchanged.')+' Render comparison required.'}
    Path(report_path).parent.mkdir(parents=True,exist_ok=True);Path(report_path).write_text(json.dumps(report,indent=2))
    return report
