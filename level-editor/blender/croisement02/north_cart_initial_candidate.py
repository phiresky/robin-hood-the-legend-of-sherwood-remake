"""Private cart-only solid hypothesis; horse-team source remains separately owned."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from scenery_geometry import Mesh
from tree_geometry import SIN,COS,RAY
from log_trap_state_candidate import point,sha,material
from render_slots import acquire,release

def main():
    root=OUT/'state-target-evidence';dest=OUT/'north-cart-initial-candidate-v2';dest.mkdir(exist_ok=False);manifest=json.loads((root/'north-cart/manifest.json').read_text());part=manifest['parts'][0];frame=part['frames'][0];source=Path(frame['image']);rgba=np.array(Image.open(source).convert('RGBA'));height,width=rgba.shape[:2];domain=Image.new('L',(width,height));polygon=[(99,41),(144,0),(204,9),(211,89),(180,122),(176,144),(149,151),(132,128),(90,120),(94,90)];ImageDraw.Draw(domain).polygon(polygon,fill=255);owned=(np.array(domain)>0)&(rgba[:,:,3]>0)&~np.all(rgba[:,:,:3]==[0,0,255],axis=2);rgba[:,:,3]=owned.astype(np.uint8)*255;imagepath=dest/'cart-owned-source.png';Image.fromarray(rgba).save(imagepath);Image.fromarray(owned.astype(np.uint8)*255).save(dest/'cart-source-domain.png')
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=12;scene.cycles.use_denoising=False;scene.view_settings.view_transform='Standard';scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.resolution_x=scene.render.resolution_y=512;scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
    mat=material(imagepath);gray=bpy.data.materials.new('Unobserved cart structure');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.17,.17,.17,1)
    # Axle centers and wheel radii are an explicit source survey, not target pivot bounds.
    left,top=[part['position'][i]+frame['offset'][i]for i in range(2)];front=point(left+126.6,top+109.1,22);u=Vector((.5,.8660254,0));v=Vector((.8660254,-.5,0));length=76;halfwidth=35;objects=[]
    def build(name,mesh):
        data=bpy.data.meshes.new(name);data.from_pydata(mesh.vertices,[],mesh.faces);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges),name;bm.to_mesh(data);bm.free();data.update();obj=bpy.data.objects.new(name,data);scene.collection.objects.link(obj);data.materials.append(mat);data.materials.append(gray);uv=data.uv_layers.new(name='Native target projection')
        for face in data.polygons:
            face.material_index=0 if face.normal.dot(RAY)>.05 else 1
            for loop in face.loop_indices:
                p=data.vertices[data.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-left)/width,1-(-p.y*SIN-p.z*COS-top)/height)
        obj['component']='cart-scenery-only';obj['geometry_status']='unapproved inferred closed volume';objects.append(obj);return obj
    def box(name,center,du,dv,dz):
        m=Mesh();m.box(center,u,v,du,dv,dz);return build(name,m)
    box('Cart bed',front+u*length/2+Vector((0,0,11)),length+12,halfwidth*2,5)
    for side in [-1,1]:
        box(f'Cart side board {side}',front+u*length/2+v*side*(halfwidth-1)+Vector((0,0,21)),length+9,3,16)
    for end in [-4,length+4]:
        box(f'Cart end board {end}',front+u*end+Vector((0,0,23)),3,halfwidth*2,20)
        for side in [-1,1]:box(f'Canopy post {end} {side}',front+u*end+v*({-1:-27,1:36}[side])+Vector((0,0,43.5)),3,3,61)
    for axle in [0,length]:
        m=Mesh();m.tube(front+u*axle-v*41,front+u*axle+v*41,2,n=10);build(f'Axle {axle}',m)
        for side in [-1,1]:
            center=front+u*axle+v*side*38
            # Rim annulus with finite width and closed inner/outer wall.
            m=Mesh();n=24
            for depth,radius in [(-2,22),(-2,18),(2,22),(2,18)]:
                for i in range(n):m.vertices.append(tuple(center+v*depth+(u*math.cos(i*math.tau/n)+Vector((0,0,math.sin(i*math.tau/n))))*radius))
            for i in range(n):
                j=(i+1)%n
                for a,b in [(0,24),(48,72),(0,48),(24,72)]:m.faces.append((a+i,a+j,b+j,b+i))
            build(f'Wheel rim {axle} {side}',m)
            m=Mesh();m.tube(center-v*4,center+v*4,4,n=12)
            for i in range(10):m.tube(center,center+(u*math.cos(i*math.tau/10)+Vector((0,0,math.sin(i*math.tau/10))))*19,1.25,n=6)
            build(f'Wheel hub and spokes {axle} {side}',m)
    # Closed shallow barrel roof, with two concentric arch skins and end/edge thickness.
    m=Mesh();n=16
    for end in [-7,length+7]:
        for thickness in [0,-1.5]:
            for i in range(n+1):
                angle=math.pi*i/n;m.vertices.append(tuple(front+u*end+v*(4.6+31.4*math.cos(angle))+Vector((0,0,74+13*math.sin(angle)+thickness))))
    stride=n+1
    for i in range(n):
        for a,b in [(0,2*stride),(stride,3*stride),(0,stride),(2*stride,3*stride)]:m.faces.append((a+i,a+i+1,b+i+1,b+i))
    for i in [0,n]:m.faces.append((i,stride+i,3*stride+i,2*stride+i))
    build('Closed barrel canopy',m)
    # Hanging fabric has finite thickness; folds and hidden reverse fabric are hypotheses.
    for side,across in [(-1,-27),(1,36)]:
        for start,end in [(-4,20),(56,80)]:
            m=Mesh();n=8
            for depth in [-.4,.4]:
                for row in [0,1]:
                    for i in range(n+1):
                        fraction=i/n;along=start+(end-start)*fraction;bottom=47+5*math.sin(math.pi*fraction);z=94 if row==0 else bottom;fold=.8*math.sin(fraction*math.tau*3);m.vertices.append(tuple(front+u*along+v*(across+depth+fold)+Vector((0,0,z-22))))
            stride=n+1
            for i in range(n):
                for a,b in [(0,stride),(2*stride,3*stride),(0,2*stride),(stride,3*stride)]:m.faces.append((a+i,a+i+1,b+i+1,b+i))
            for i in [0,n]:m.faces.append((i,stride+i,3*stride+i,2*stride+i))
            build(f'Canopy curtain {side} {start}',m)
    camera_data=bpy.data.cameras.new('Review camera');camera_data.type='ORTHO';camera_data.clip_end=10000;camera=bpy.data.objects.new('Review camera',camera_data);scene.collection.objects.link(camera);scene.camera=camera;lightdata=bpy.data.lights.new('Sun','SUN');lightdata.energy=2;light=bpy.data.objects.new('Sun',lightdata);scene.collection.objects.link(light);light.rotation_euler=(.6,-.5,-.4)
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));acquire()
    try:
        for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
            center=point(left+150,top+77,0)if view=='source'else front+u*length/2+Vector((0,0,28));camera.location=center+direction*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();camera_data.ortho_scale=200
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=gray if mode=='solid'else None;scene.render.filepath=str(dest/f'{view}-{mode}.png');bpy.ops.render.render(write_still=True)
    finally:release()
    report=dict(status='first unapproved cart-only hypothesis; source and solid review required',model_sha256=sha(dest/'worker.blend'),source_sha256=sha(source),source_frame=frame,source_position=part['position'],cart_domain_polygon=polygon,cart_source_pixels=int(owned.sum()),objects=[o.name for o in objects],limitations=['Horses, harness and source shadows remain separate preserved sources; no actor geometry claim.','Four wheels, ten spokes, barrel roof depth and hidden body inferred; source-facing geometry must be reviewed.','Initial target source only; mobile approach, collapse, debris and applied endpoint missing.','No terrain support claim, state approval, generated textures or scene integration.']);(dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
