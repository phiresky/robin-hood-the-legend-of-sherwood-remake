"""Private closed sign geometry with native pose timing and mission-only instances."""
import sys,math,json
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender'),str(ROOT/'level-editor/refinement')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    directory=OUT/'state-sign-candidate/geometry-v2';directory.mkdir(exist_ok=False)
    manifest=OUT/'state-target-evidence/manifest.json';data=json.loads(manifest.read_text());profile=next(r for r in data['profiles'] if r['id']=='TG_Panel-12');frames=profile['rows'][0]['frames'];instances=[r for r in data['instances'] if r['profile_id']==profile['id']]
    assert len(instances)==5 and all(r['mission']=='S03_FoB_MP' for r in instances)
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.fps=25;scene.frame_start=1;scene.frame_end=64
    materials={}
    for index in (5,13,21,29):
        f=frames[index];path=Path(f['image']);assert sha(path)==f['image_sha256'];rgba=np.asarray(Image.open(path).convert('RGBA')).copy();known=(rgba[:,:,3]>0)&(rgba[:,:,:3].max(axis=2)>12);rgba[~known]=[115,115,115,255];rgba[:,:,3]=255;texture=directory/f'observed-pose-{index}.png';Image.fromarray(rgba).save(texture)
        mat=bpy.data.materials.new(f'Native sign pose{index} observed/unknown gray');mat.use_nodes=True;n=mat.node_tree.nodes;p=n.get('Principled BSDF');p.inputs['Roughness'].default_value=1;tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(texture));tex.image.pack();tex.interpolation='Closest';tex.extension='CLIP';mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.17,.17,.17,1);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],p.inputs['Base Color']);mat.node_tree.links.new(mix.outputs[0],p.inputs['Emission Color']);p.inputs['Emission Strength'].default_value=1;materials[index]=mat
    unknown=bpy.data.materials.new('Unobserved sign edge gray');unknown.diffuse_color=(.4,.4,.4,1)
    # The broad rear view defines the irregular board outline. Thickness is inferred.
    outline=[(-19,-36),(16,-36),(16,-32),(18,-32),(18,-31),(19,-31),(19,-27),(20,-27),(20,-24),(19,-24),(19,-18),(8,-18),(8,-17),(3,-17),(3,-18),(-19,-18),(-19,-22),(-20,-22),(-20,-30),(-19,-30)]
    vertices=[(x,y,(-v+SIN*1.4)/COS) for y in (-1.4,1.4) for x,v in outline];count=len(outline);faces=[tuple(reversed(range(count))),tuple(range(count,count*2))]+[(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
    def mesh(name,verts,faces):
        m=bpy.data.meshes.new(name);m.from_pydata(verts,[],faces);m.update();o=bpy.data.objects.new(name,m);scene.collection.objects.link(o);bm=bmesh.new();bm.from_mesh(m);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(m);bm.free();return o
    board=mesh('Signboard closed irregular timber',vertices,faces)
    post=mesh('Signpost closed timber',[(-2,-4,0),(2,-4,0),(2,-.5,0),(-2,-.5,0),(-2,-4,46),(2,-4,46),(2,-.5,46),(-2,-.5,46)],[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
    def texture_object(obj):
        for index in (5,13,21,29):obj.data.materials.append(materials[index])
        uv=obj.data.uv_layers.new(name='Native observed pose projection')
        for polygon in obj.data.polygons:
            normal=polygon.normal;index=(29 if normal.y>0 else 13) if abs(normal.y)>=abs(normal.x) else (21 if normal.x>0 else 5);polygon.material_index=(5,13,21,29).index(index);theta=(13-index)*math.tau/32;c,s=math.cos(theta),math.sin(theta);f=frames[index];ox,oy=f['offset'];w,h=f['size']
            for loop in polygon.loop_indices:
                p=obj.data.vertices[obj.data.loops[loop].vertex_index].co;x=p.x*c-p.y*s;y=p.x*s+p.y*c;u=x-ox;v=-SIN*y-COS*p.z-oy;uv.data[loop].uv=(u/w,1-v/h)
        obj['mission_profile']='Panneau';obj['asset_group']='croisement02-mission-signpost';obj['source_node']='mission-signpost';obj['inference']='Closed board thickness and unseen edge shape inferred; native frame color projected per surface.'
    for o in (board,post):texture_object(o)
    prototype=[board,post];all_instances=[]
    for i,record in enumerate(instances):
        target=record['target'];parent=bpy.data.objects.new(f'Mission sign target{record["target_index"]}',None);scene.collection.objects.link(parent);parent.location=(target['position_x'],-target['position_y']/SIN,0);parent['mission_visibility']='S03_FoB_MP';parent['target_index']=record['target_index'];parent['native_action']=target['action'];parent['native_layer']=target['layer']
        for original in prototype:
            obj=original if i==0 else bpy.data.objects.new(original.name+f' target{record["target_index"]}',original.data);obj.parent=parent
            if i:scene.collection.objects.link(obj)
        for f in range(33):parent.rotation_euler.z=(13-f)*math.tau/32;parent.keyframe_insert(data_path='rotation_euler',frame=1+2*f)
        all_instances.append(parent)
    for action in bpy.data.actions:
        for layer in action.layers:
            for strip in layer.strips:
                for bag in strip.channelbags:
                    for curve in bag.fcurves:
                        for key in curve.keyframe_points:key.interpolation='CONSTANT'
    scene.frame_set(1)
    audits=[]
    for o in prototype:
        bm=bmesh.new();bm.from_mesh(o.data);audits.append(dict(object=o.name,faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces)));bm.free()
    assert all(not r['nonmanifold_edges'] and not r['degenerate_faces'] for r in audits)
    bpy.ops.wm.save_as_mainfile(filepath=str(directory/'model.blend'))
    # Review a local prototype; five saved placements remain unchanged in the model.
    for p in all_instances[1:]:
        for child in p.children:child.hide_render=True
    parent=all_instances[0];parent.location=(0,0,0)
    camera_data=bpy.data.cameras.new('Native sign review camera');camera_data.type='ORTHO';camera_data.ortho_scale=70;camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera);target=Vector((0,0,23));camera.location=target+RAY*500;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.resolution_x=280;scene.render.resolution_y=280;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    world=bpy.data.worlds.new('Sign review world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs[0].default_value=(.3,.3,.3,1);world.node_tree.nodes['Background'].inputs[1].default_value=.5;scene.world=world
    lightdata=bpy.data.lights.new('Sign review softbox','AREA');lightdata.energy=18000;lightdata.shape='DISK';lightdata.size=100;light=bpy.data.objects.new(lightdata.name,lightdata);scene.collection.objects.link(light);light.location=(-80,-120,150);light.rotation_euler=(target-light.location).to_track_quat('-Z','Y').to_euler()
    solid=bpy.data.materials.new('Solid review');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.5,.5,.5,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=1
    for mode in ('actual','solid'):
        scene.view_layers[0].material_override=solid if mode=='solid' else None
        sheet=Image.new('RGB',(1120,600),(75,75,75));draw=ImageDraw.Draw(sheet)
        for i,frame in enumerate(range(0,32,4)):
            scene.frame_set(1+frame*2);scene.render.filepath=str(directory/f'{mode}-pose-{frame:02}.png');bpy.ops.render.render(write_still=True);im=Image.open(scene.render.filepath).convert('RGBA');sheet.paste(im,(i%4*280,i//4*300+20),im);draw.text((i%4*280+5,i//4*300+5),f'Native pose{frame}',fill='white')
        sheet.save(directory/f'{mode}8.png')
    write_json(directory/'evidence.json',dict(status='private first closed sign geometry; visual review pending',model_sha256=sha(directory/'model.blend'),source_manifest_sha256=sha(manifest),profile=profile,instances=instances,topology=audits,timing=dict(ticks_per_second=25,ticks_per_frame=2,frames=32,period_ticks=64),notes=['Five separately positioned mission-only instances. No catalog/runtime mutation.','Shadow pixels excluded from solid geometry and retained in source evidence only.','Geometry and turn rate inferred from native full rotation; actions0/210/211 retained separately in evidence.','Read-only review removes other instances and recenters prototype after saving model.']))
    print(directory)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
