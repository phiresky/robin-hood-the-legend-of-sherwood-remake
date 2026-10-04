"""Private volumetric net bag and counterweight hypotheses, separate from rigging support."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from tree_geometry import RAY,SIN,COS
from scenery_geometry import Mesh
from log_trap_state_candidate import point,material,sha
from render_slots import acquire,release

def main():
    dest=OUT/'net-endpoint-bodies-v2';dest.mkdir(exist_ok=False)
    source=OUT/'net-state-bindings-v1/manifest.json';assembly=json.loads(source.read_text())['assemblies'][0]
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.use_denoising=False;scene.view_settings.view_transform='Standard';scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.resolution_x=scene.render.resolution_y=512
    scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
    gray=bpy.data.materials.new('Unobserved net body surface');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.17,.17,.17,1)
    groups={};records=[]
    for suffix in ['e','i']:
        patch=next(p for p in assembly['patches']if p['name'].endswith(suffix));frame=patch['states']['final']['frames'][0];src=OUT/'source-states'/frame['image'];rgba=np.array(Image.open(src).convert('RGBA'));rgba[np.all(rgba[:,:,:3]==[0,0,255],axis=2),3]=0;image=dest/f'{suffix}-owned-source.png';Image.fromarray(rgba).save(image);mat=material(image);left,top,width,height=frame['bbox'];objects=[]
        def build(name,m):
            mesh=bpy.data.meshes.new(name);mesh.from_pydata(m.vertices,[],m.faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();mesh.update();o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o);mesh.materials.append(mat);mesh.materials.append(gray);uv=mesh.uv_layers.new(name='Native target projection')
            for f in mesh.polygons:
                f.material_index=0 if f.normal.dot(RAY)>.05 else 1
                for loop in f.loop_indices:
                    p=mesh.vertices[mesh.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-left)/width,1-(-p.y*SIN-p.z*COS-top)/height)
            o['state_variant']=suffix;o['geometry_status']='unapproved endpoint body hypothesis; rigging incomplete';objects.append(o);return o
        # The bag has round horizontal sections, inferred depth equal to width.
        # Height follows the lifted silhouette above the native ground waypoint.
        rings=[(56,1),(60,5),(71,10),(85,14),(98,10),(109,7),(116,3),(121,1)]
        if suffix=='i':rings=[(47,1),(52,8),(63,16),(79,23),(96,17),(109,9),(116,3),(121,1)]
        center_y=-1113/SIN;m=Mesh();n=24
        for z,radius in rings:
            for i in range(n):
                angle=i*math.tau/n;cx=1334 if suffix=='e' else 1333;m.vertices.append((cx+radius*math.cos(angle),center_y+radius*math.sin(angle),z))
        m.faces.append(tuple(reversed(range(n))))
        for row in range(len(rings)-1):
            for i in range(n):j=(i+1)%n;m.faces.append((row*n+i,row*n+j,(row+1)*n+j,(row+1)*n+i))
        m.faces.append(tuple((len(rings)-1)*n+i for i in range(n)));build(f'Net01 {suffix} closed bag',m)
        m=Mesh();m.tube(point(1303,1043,85.5),point(1322,1034,85.5),9,n=16);build(f'Net01 {suffix} counterweight log',m)
        groups[suffix]=objects;records.append(dict(variant=suffix,source_frame=frame,source_sha256=sha(src),objects=[o.name for o in objects],depth_assumption='circular horizontal bag sections; world depth equals width',ground_waypoint=patch['source_state']['waypoint']))
    data=bpy.data.cameras.new('Review camera');data.type='ORTHO';data.clip_end=10000;camera=bpy.data.objects.new('Review camera',data);scene.collection.objects.link(camera);scene.camera=camera;sun=bpy.data.lights.new('Sun','SUN');sun.energy=2;o=bpy.data.objects.new('Sun',sun);scene.collection.objects.link(o);o.rotation_euler=(.6,-.5,-.4)
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));acquire()
    try:
        for suffix,objects in groups.items():
            for k,others in groups.items():
                for o in others:o.hide_render=k!=suffix
            center=point(1330,1040,85);data.ortho_scale=130
            for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.7)).normalized())]:
                camera.location=center+direction*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
                for mode in ['actual','solid']:
                    scene.view_layers[0].material_override=gray if mode=='solid'else None;scene.render.filepath=str(dest/f'{suffix}-{view}-{mode}.png');bpy.ops.render.render(write_still=True)
    finally:release()
    result=dict(status='private bag/counterweight body hypothesis only; not complete trap geometry',source_manifest_sha256=sha(source),model_sha256=sha(dest/'worker.blend'),variants=records,limitations=['Suspension ropes, pulleys and support attachment are not yet modeled; body heights are inferred from source projection above ground waypoint.','Bag fabric thickness/weave and folds not reconstructed; closed volume tests endpoint silhouette and plausible depth only.','Only phase0 e/i endpoints; all14 native final phases and lifting motion remain preserved source requirements.','No actor replacement, texture fill, geometry approval or scene integration.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
if __name__=='__main__':main()
