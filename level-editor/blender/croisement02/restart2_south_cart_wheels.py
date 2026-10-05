"""Build a private terminal wheel-pair hypothesis with bounded source ownership."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from scenery_geometry import Mesh
from tree_geometry import SIN,COS,RAY
from log_trap_state_candidate import point,sha,material
from render_slots import acquire,release

def main():
    dest=OUT/'restart2-state/south-cart-wheel-pair-v2';dest.mkdir(exist_ok=False)
    source_manifest=OUT/'state-target-evidence/south-cart/manifest.json';m=json.loads(source_manifest.read_text());part=m['parts'][1];frame=part['frames'][-1];source=Path(frame['image']);assert sha(source)==frame['image_sha256'];rgba=np.array(Image.open(source).convert('RGBA'))
    # The terminal wheel/axle assembly is detached from the surviving fence at x=250.
    # This bounded review region is a hypothesis, not a semantic ownership label.
    domain=np.indices(rgba.shape[:2])[1]>=250;domain&=rgba[:,:,3]>0;domain&=~np.all(rgba[:,:,:3]==[0,0,255],axis=2);rgba[:,:,3]=domain.astype(np.uint8)*255;paint=dest/'wheel-pair-source.png';Image.fromarray(rgba).save(paint)
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=12;scene.cycles.use_denoising=False;scene.view_settings.view_transform='Standard';scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.resolution_x=scene.render.resolution_y=512;scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
    mat=material(paint);gray=bpy.data.materials.new('Unobserved wheel reverse');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.2,.2,.2,1)
    left,top=[part['position'][i]+frame['offset'][i]for i in range(2)];objects=[];records=[]
    def build(name,mesh):
        data=bpy.data.meshes.new(name);data.from_pydata(mesh.vertices,[],mesh.faces);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.to_mesh(data);bm.free();data.update();obj=bpy.data.objects.new(name,data);scene.collection.objects.link(obj);data.materials.append(mat);data.materials.append(gray);uv=data.uv_layers.new(name='Native target projection')
        for face in data.polygons:
            face.material_index=0 if face.normal.dot(RAY)>.05 else 1
            for loop in face.loop_indices:
                p=data.vertices[data.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-left)/rgba.shape[1],1-(-p.y*SIN-p.z*COS-top)/rgba.shape[0])
        obj['state']='south cart terminal debris';obj['source_profile']=part['profile_id'];obj['status']='private inferred solid; no approval';objects.append(obj);records.append(dict(name=name,closed_manifold=True,positive_volume=volume));return obj
    centers=[]
    for i,(x,y,radius)in enumerate([(279,80,20),(340,101,22)]):
        center=point(left+x,top+y,4);centers.append(center)
        mesh=Mesh();mesh.tube(center-Vector((0,0,4)),center+Vector((0,0,0)),radius,n=32);build(f'Fallen plank wheel {i+1}',mesh)
        mesh=Mesh();mesh.tube(center,center+Vector((0,0,6)),4,n=12);build(f'Raised wheel hub {i+1}',mesh)
    a=point(left+279,top+86,2);b=point(left+340,top+112,2);mesh=Mesh();mesh.tube(a,b,2,n=8);build('Broken axle joining fallen wheels',mesh)
    lightdata=bpy.data.lights.new('Sun','SUN');lightdata.energy=2;light=bpy.data.objects.new('Sun',lightdata);scene.collection.objects.link(light);light.rotation_euler=(.6,-.5,-.4)
    cd=bpy.data.cameras.new('Review camera');cd.type='ORTHO';cd.clip_end=10000;camera=bpy.data.objects.new('Review camera',cd);scene.collection.objects.link(camera);scene.camera=camera
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));center=(centers[0]+centers[1])/2;images=[];acquire()
    try:
        for i in range(8):
            angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));camera.location=center+direction*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();cd.ortho_scale=150
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=gray if mode=='solid'else None;path=dest/f'{i:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);images.append(dict(view=i,mode=mode,path=path.name,sha256=sha(path)))
    finally:release()
    for mode in ['actual','solid']:
        sheet=Image.new('RGB',(4*256,2*256),(40,40,40))
        for i in range(8):
            im=Image.open(dest/f'{i:02}-{mode}.png').convert('RGBA').resize((256,256));sheet.paste(im,((i%4)*256,(i//4)*256),im)
        sheet.save(dest/f'{mode}-sheet.png')
    report=dict(status='Private terminal wheel-pair hypothesis; source, terrain contact and full-cart review pending',model_sha256=sha(dest/'worker.blend'),source_manifest_sha256=sha(source_manifest),source_frame=frame,native_position=part['position'],source_domain_pixels=int(domain.sum()),source_region='x >= 250; reserved shadow key excluded',geometry=records,review_images=images,limitations=['Horizontal plank wheels, thickness and hidden backs are inferred from the terminal face ellipses.','Ground height zero is a provisional placement requiring exact terrain contact verification.','No horse, fence or cart body geometry ownership claimed.','No transition correspondence, complete state or texture-fill approval.'])
    (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
