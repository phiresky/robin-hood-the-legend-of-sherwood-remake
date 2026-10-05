"""Render saved bridge against explicit provisional land, water and riverbed."""
import json,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector,Matrix
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_bridge import OUT,ASSET,RUNS,world
from render_slots import acquire,release
from render_views import render_views
from texture_camera import depth_clip_range
from evidence_io import sha,write_json

def main():
    worker=OUT/'restart2/bridge-v6/assets'/ASSET;out=OUT/'restart2/bridge-contact-v2';out.mkdir(parents=True,exist_ok=False)
    acquire();digest=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    old=bpy.data.collections['Croisement03 Working'];scene=bpy.data.scenes.new('Bridge land and water contact');bpy.context.window.scene=scene;objects=[]
    for o in old.all_objects:
        if o.type!='MESH' or o.get('asset_group')!=ASSET:continue
        copy=o.copy();copy.parent=None;copy.matrix_world=o.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy);objects.append(copy)
    geometry=[o.matrix_world@v.co for o in objects for v in o.data.vertices];bed_z=min(v.z for v in geometry)+.15
    corners=[world(p,0) for p in RUNS['deck_perimeter']];axis=((corners[2]+corners[3])-(corners[0]+corners[1])).normalized();across=(corners[1]-corners[0]).normalized()
    def material(name,color,alpha=1):
        m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;l=m.node_tree.links;bs=n.get('Principled BSDF');bs.inputs['Base Color'].default_value=(*color,1);bs.inputs['Roughness'].default_value=.9
        if alpha<1:
            transparent=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');mix.inputs[0].default_value=alpha;l.new(transparent.outputs[0],mix.inputs[1]);l.new(bs.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],n.get('Material Output').inputs['Surface'])
        return m
    soil=material('Inferred neutral bank',(.27,.24,.17));bed=material('Unobserved riverbed',(.14,.12,.08));water=material('Inferred water level',(.11,.20,.15),.62)
    def mesh(name,vertices,faces,mat):
        data=bpy.data.meshes.new(name);data.from_pydata([tuple(v) for v in vertices],[],faces);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(data);bm.free();o=bpy.data.objects.new(name,data);scene.collection.objects.link(o);data.materials.append(mat);objects.append(o);return o
    bank_faces=[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)]
    edges=[]
    for label,a,b,direction in [('Northwest',corners[0],corners[1],axis),('Southeast',corners[3],corners[2],-axis)]:
        a=a-across*40;b=b+across*40;edges.append((a,b));top=[a-direction*60,b-direction*60,b,a];bottom=[Vector((p.x,p.y,bed_z)) for p in top]
        bottom[2]+=direction*25;bottom[3]+=direction*25
        mesh(label+' provisional bank',top+bottom,bank_faces,soil)
    water_corners=[edges[0][0],edges[0][1],edges[1][1],edges[1][0]]
    mesh('Water surface below deck',[Vector((p.x,p.y,-35)) for p in water_corners],[(0,1,2,3)],water)
    mesh('Riverbed contact below water',[Vector((p.x,p.y,bed_z)) for p in water_corners],[(0,1,2,3)],bed)
    sun=bpy.data.lights.new('Contact inspection sun','SUN');sun.energy=2;light=bpy.data.objects.new(sun.name,sun);scene.collection.objects.link(light);light.rotation_euler=Vector((-.45,-.55,.70)).to_track_quat('Z','Y').to_euler();scene.world=bpy.data.worlds.new('Contact inspection world');scene.world.color=(.18,.18,.18)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.cycles.use_denoising=False;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    packet=json.loads((worker/'modified/views.json').read_text());points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];views={}
    for v in packet['views']:
        data=bpy.data.cameras.new('Bridge contact '+str(v['index']));data.type='ORTHO';data.ortho_scale=v['ortho_scale']*1.65;inverse=Matrix(v['camera_matrix_world']).inverted();data.clip_start,data.clip_end=depth_clip_range(-(inverse@p).z for p in points);camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.matrix_world=Matrix(v['camera_matrix_world']);views[f'view-{v["index"]}']=camera.name
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'joint.blend'))
    render_views(scene.name,views,out,modes=('textured',),width=384)
    images=[Image.open(out/f'view-{i}-textured.png').convert('RGB') for i in range(8)];tw,th=images[0].size;sheet=Image.new('RGB',(tw*4,th*2))
    for i,im in enumerate(images):sheet.paste(im,((i%4)*tw,(i//4)*th))
    sheet.save(out/'sheet.png');write_json(out/'evidence.json',dict(model_sha256=digest,joint_sha256=sha(out/'joint.blend'),sheet_sha256=sha(out/'sheet.png'),deck_z=0,landing_top_z=[0,0],water_z=-35,bed_z=bed_z,pier_bed_penetration=.15,limitations=['Bank slope, waterZ-35 and bed depth are private inferred geometry. They are not measured native heights.','Deck lands on the native zero-elevation frame. Riverbed depth was selected to seat the reconstructed pier, not independently observed.','No flat plane is hidden inside this joint scene: the actual bank solids, water and bed are rendered together.','Full-map terrain and native shore rocks remain unfinished. The existing global flat receiver must be replaced in the final integration.','Below-ground overlap between bridge end thickness and bank solids represents intentional embedded landing support.']))
    if sha(worker/'model.blend')!=digest:raise ValueError('Approved worker changed during contact proof')
    release()

if __name__=='__main__':main()
