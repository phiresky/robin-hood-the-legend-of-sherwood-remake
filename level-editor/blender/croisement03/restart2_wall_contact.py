"""Review stone-wall ground contact and coarse native tree neighbors."""
import argparse
import json
import sys
from pathlib import Path
import bpy
from mathutils import Matrix,Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from evidence_io import sha,write_json
from render_views import render_views
from texture_camera import depth_clip_range

def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);w=args.workspace.resolve();out=w/'inspection/ground-contact';out.mkdir(exist_ok=False)
    acquire();digest=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
    points=[o.matrix_world@v.co for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name for v in o.data.vertices];root=Vector(tuple((min(v[i] for v in points)+max(v[i] for v in points))/2 for i in range(3)));ground=0.0;root.z=ground
    working=bpy.data.collections['Croisement03 Working'];targets=[o for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==w.name]
    scene=bpy.data.scenes.new('Croisement03 wall ground contact');bpy.context.window.scene=scene;selected=[];neighbors=[]
    for obj in list(working.all_objects):
        if obj.type!='MESH' or obj.get('source_node')=='ground':continue
        points=[obj.matrix_world@v.co for v in obj.data.vertices]
        near=(min(p.x for p in points)-60<=root.x<=max(p.x for p in points)+60 and min(p.y for p in points)-60<=root.y<=max(p.y for p in points)+60)
        if obj not in targets and not near:continue
        copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy);selected.append(copy)
        if obj not in targets:neighbors.append(obj['source_node'])
    plane=bpy.data.meshes.new('Native flat ground contact patch');s=150;plane.from_pydata([(root.x-s,root.y-s,ground),(root.x+s,root.y-s,ground),(root.x+s,root.y+s,ground),(root.x-s,root.y+s,ground)],[],[(0,1,2,3)]);plane.update();obj=bpy.data.objects.new(plane.name,plane);scene.collection.objects.link(obj);selected.append(obj)
    material=bpy.data.materials.new('Neutral ground contact diagnostic');material.use_nodes=True;material.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.22,.25,.19,1);material.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1;plane.materials.append(material)
    sun=bpy.data.lights.new('Contact inspection sun','SUN');sun.energy=2;light=bpy.data.objects.new(sun.name,sun);scene.collection.objects.link(light);light.rotation_euler=Vector((-.45,-.55,.70)).to_track_quat('Z','Y').to_euler();scene.world=bpy.data.worlds.new('Contact inspection world');scene.world.color=(.18,.18,.18)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100
    packet=json.loads((w/'modified/views.json').read_text());points=[o.matrix_world@v.co for o in selected for v in o.data.vertices];views={}
    for v in packet['views']:
        data=bpy.data.cameras.new('Contact '+str(v['index']));data.type='ORTHO';data.ortho_scale=v['ortho_scale']*1.5;inverse=Matrix(v['camera_matrix_world']).inverted();data.clip_start,data.clip_end=depth_clip_range(-(inverse@p).z for p in points);camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.matrix_world=Matrix(v['camera_matrix_world']);views[f'view-{v["index"]}']=camera.name
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'joint.blend'))
    render_views(scene.name,views,out,modes=('textured',),width=384)
    images=[Image.open(out/f'view-{i}-textured.png').convert('RGB') for i in range(8)];tw,th=images[0].size;sheet=Image.new('RGB',(tw*4,th*2))
    for i,im in enumerate(images):sheet.paste(im,((i%4)*tw,(i//4)*th))
    sheet.save(out/'sheet.png');write_json(out/'evidence.json',dict(model_sha256=digest,sheet_sha256=sha(out/'sheet.png'),root_world=list(root),ground_z=ground,neighbor_nodes=neighbors,limitations=['Ground is a neutral diagnostic plane at the native reconstruction height. This does not complete terrain ownership or fill its hidden appearance.','Nearby wood is unchanged coarse native context, not approved refinement.','Refined neighboring tree, ground ownership and source foliage occlusion remain unfinished.']))
    if sha(w/'model.blend')!=digest:raise ValueError('Model changed during contact review')
    release();print(out/'sheet.png')
if __name__=='__main__':main()
