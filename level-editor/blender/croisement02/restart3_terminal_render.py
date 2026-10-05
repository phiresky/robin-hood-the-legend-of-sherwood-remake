"""Source-masked terminal review with partially visible triangle sampling."""
from restart3_cart_cargo_debris import *

def finish_terminal(scene,gray,dest,box,metadata):
    objects=[o for o in scene.objects if o.type=='MESH'];vertices=[];faces=[]
    for obj in objects:
        start_index=len(vertices);vertices.extend(v.co.copy() for v in obj.data.vertices)
        faces.extend(tuple(start_index+i for i in p.vertices) for p in obj.data.polygons)
    bvh=BVHTree.FromPolygons(vertices,faces)
    for obj in objects:
        for face in obj.data.polygons:
            # Boundary triangles may be partly visible although their centroid is occluded.
            # Material alpha still limits RGB to the independently traced source role.
            points=[face.center]+[face.center.lerp(obj.data.vertices[i].co,.8) for i in face.vertices]
            visible=False
            if face.normal.dot(RAY)>.05:
                for probe in points:
                    hit=bvh.ray_cast(probe+RAY*2000,-RAY,4000)
                    if hit[0] is not None and (hit[0]-probe).length<.05:
                        visible=True;break
            face.material_index=0 if visible else 1
    bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'),compress=True)
    center=Vector(tuple((min(p[i] for p in vertices)+max(p[i] for p in vertices))/2 for i in range(3)))
    camera=scene.camera;camera.data.ortho_scale=max((max(p[i] for p in vertices)-min(p[i] for p in vertices)) for i in range(3))*1.65
    scene.render.resolution_x=scene.render.resolution_y=384
    for index in range(8):
        az=math.radians(index*45);direction=Vector((math.sin(az)*COS,-math.cos(az)*COS,SIN))
        camera.location=center+direction*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
        for mode in ['actual','solid']:
            scene.view_layers[0].material_override=gray if mode=='solid' else None
            scene.render.filepath=str(dest/f'view-{index}-{mode}.png');bpy.ops.render.render(write_still=True)
    for mode in ['actual','solid']:
        sheet=Image.new('RGBA',(1536,768))
        for index in range(8):sheet.paste(Image.open(dest/f'view-{index}-{mode}.png'),((index%4)*384,(index//4)*384))
        sheet.save(dest/f'{mode}.png')
    target=point((box[0]+box[2])/2,(box[1]+box[3])/2,0)
    camera.location=target+RAY*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler()
    camera.data.ortho_scale=box[2]-box[0]
    scene.render.resolution_x=(box[2]-box[0])*6;scene.render.resolution_y=(box[3]-box[1])*6
    for mode in ['actual','solid']:
        scene.view_layers[0].material_override=gray if mode=='solid' else None
        scene.render.filepath=str(dest/f'native-{mode}.png');bpy.ops.render.render(write_still=True)
    metadata.update(model_sha256=sha(dest/'worker.blend'),source_box=box,components=[o.name for o in objects],
                    native_first=True,recipe=record_recipe(dest,Path(__file__)))
    write_json(dest/'manifest.json',metadata)

