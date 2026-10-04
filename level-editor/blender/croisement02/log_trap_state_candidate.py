"""Build bounded solid log-trap endpoint candidates from native target artwork."""
import json,math,sys,hashlib
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from render_slots import acquire,release
from scenery_geometry import Mesh
from tree_geometry import SIN,COS,RAY

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def point(x,y,z):return Vector((x,-(y+z*COS)/SIN,z))
def material(imagepath):
    material=bpy.data.materials.new('Native endpoint RGB; unobserved sides gray');material.use_nodes=True;n=material.node_tree.nodes;n.clear();l=material.node_tree.links
    texture=n.new('ShaderNodeTexImage');texture.image=bpy.data.images.load(str(imagepath));texture.image.pack();texture.interpolation='Closest';texture.extension='CLIP';uv=n.new('ShaderNodeUVMap');uv.uv_map='Native target projection';l.new(uv.outputs['UV'],texture.inputs['Vector'])
    mix=n.new('ShaderNodeMixRGB');mix.blend_type='MIX';mix.inputs[1].default_value=(.17,.17,.17,1);l.new(texture.outputs['Alpha'],mix.inputs[0]);l.new(texture.outputs['Color'],mix.inputs[2]);shader=n.new('ShaderNodeBsdfPrincipled');shader.inputs['Roughness'].default_value=1;l.new(mix.outputs[0],shader.inputs['Base Color']);l.new(mix.outputs[0],shader.inputs['Emission Color']);shader.inputs['Emission Strength'].default_value=.35;out=n.new('ShaderNodeOutputMaterial');l.new(shader.outputs[0],out.inputs[0]);return material

def main():
    source=OUT/'state-target-evidence/log-trap';manifest=json.loads((source/'manifest.json').read_text());dest=OUT/'log-trap-state-candidate-v8';dest.mkdir(exist_ok=False);box=manifest['bbox'];left,top,right,bottom=box
    # Endpoint centers are surveyed in their own native sprite coordinates.
    initial=json.loads((source/'covered-cylinder-fit.json').read_text())['survey']
    terminal=json.loads((source/'applied-coherent-bank-slopes-v2.json').read_text())['survey']
    acquire()
    try:
        bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Croisement02 log trap endpoints';scene.render.engine='CYCLES';scene.cycles.samples=24;scene.view_settings.view_transform='Standard';scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
        lightdata=bpy.data.lights.new('Sun','SUN');lightdata.energy=2;light=bpy.data.objects.new('Sun',lightdata);scene.collection.objects.link(light);light.rotation_euler=(.6,-.5,-.4)
        states={};audits=[]
        for name,tick,survey,origin in [('covered',-1,initial,(505,453)),('applied',89,terminal,(left,top))]:
            image=source/f'tick-{tick:03d}.png';mat=material(image);objects=[]
            gray=bpy.data.materials.new('Unobserved log end and bark');gray.diffuse_color=(.17,.17,.17,1);gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.17,.17,.17,1)
            for index,row in enumerate(survey):
                ax,ay,bx,by,radius,z,*other_height=row;end_z=other_height[0]if other_height else z
                m=Mesh();m.tube(point(origin[0]+ax,origin[1]+ay,z),point(origin[0]+bx,origin[1]+by,end_z),radius,n=16);mesh=bpy.data.meshes.new(f'{name} log {index:02d}');mesh.from_pydata(m.vertices,[],m.faces);mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj);mesh.materials.append(mat);mesh.materials.append(gray)
                bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert not any(not e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();mesh.update();uv=mesh.uv_layers.new(name='Native target projection')
                for face in mesh.polygons:
                    face.material_index=0 if face.normal.dot(RAY)>.05 else 1
                    for loop in face.loop_indices:
                        p=mesh.vertices[mesh.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-left)/(right-left),1-(-p.y*SIN-p.z*COS-top)/(bottom-top))
                mesh.update()
                obj['state_endpoint']=name;obj['native_source_tick']=tick;obj['geometry_status']='unapproved bounded candidate';objects.append(obj)
            states[name]=objects
        camera_data=bpy.data.cameras.new('Review camera');camera_data.type='ORTHO';camera_data.clip_end=10000;camera=bpy.data.objects.new('Review camera',camera_data);scene.collection.objects.link(camera);scene.camera=camera;scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.resolution_percentage=100
        allpoints=[v.co for objects in states.values() for obj in objects for v in obj.data.vertices];center=Vector(tuple((min(p[i]for p in allpoints)+max(p[i]for p in allpoints))/2 for i in range(3)))
        for name,objects in states.items():
            for key,others in states.items():
                for obj in others:obj.hide_render=key!=name
            for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
                target=point((left+right)/2,(top+bottom)/2,0) if view=='source' else center
                camera.location=target+direction*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();camera_data.ortho_scale=max(right-left,bottom-top)*1.2 if view=='source' else 400
                for mode in ['actual','solid']:
                    replacements=[]
                    if mode=='solid':
                        for obj in objects:
                            for slot in obj.material_slots:replacements.append((slot,slot.material));slot.material=gray
                    scene.render.filepath=str(dest/f'{name}-{view}-{mode}.png');bpy.ops.render.render(write_still=True)
                    for slot,original in replacements:slot.material=original
            audits.append(dict(state=name,objects=len(objects),faces=sum(len(o.data.polygons)for o in objects),closed_manifold=True,native_tick=objects[0]['native_source_tick']))
        for obj in states['applied']:obj.hide_render=True
        for obj in states['covered']:obj.hide_render=False
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
        sheet=Image.new('RGB',(1024,1024),'#222222')
        for row,name in enumerate(states):
            for col,view in enumerate(['source','oblique']):
                image=Image.open(dest/f'{name}-{view}-actual.png').convert('RGBA');sheet.paste(image,(col*512,row*512),image)
        sheet.save(dest/'comparison.png')
        report=dict(status='candidate requires self-review and geometry refinement',source_manifest_sha256=sha(source/'manifest.json'),model_sha256=sha(dest/'worker.blend'),support_manifest_sha256=sha(source/'applied-coherent-bank-slopes-v2.json'),surveys=dict(initial=initial,applied=terminal),geometry=audits,limitations=['Applied full-log slopes use audited bank support with fixed source endpoints; contacts and foreground visibility still require actual geometry review.','Covered and applied log counts are independent hypotheses, not matched physical identities or an animation rig.','Endpoint candidates only; per-log correspondence and native transition geometry (last target reaches terminal at tick87) not implemented.','Log end centers and radii inferred from native source; terminal fragments not assigned fabricated identities.','Native atlas RGB on source-facing solid surfaces; unobserved sides intentionally gray, no texture generation requested.','No permanent catalog or scene integration; source shadow patch remains separate ground state.'])
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    finally:release()
if __name__=='__main__':main()
