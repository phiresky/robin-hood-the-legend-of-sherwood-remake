"""Private fallen-log contact with inferred bank and source-sized support rock."""
import json,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector,Matrix
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_fallen_log import OUT,ASSET,SIN,COS
import math
def world(pixel,z):
    x,y=pixel;return Vector((x,-(y+z*COS)/SIN,z))
from render_slots import acquire,release
from render_views import render_views
from texture_camera import depth_clip_range
from evidence_io import sha,write_json

def main():
    worker=OUT/'restart2/fallen-log-v2/assets'/ASSET;out=OUT/'restart2/fallen-log-contact-v2';out.mkdir(parents=True,exist_ok=False)
    acquire();digest=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    old=bpy.data.collections['Croisement03 Working'];scene=bpy.data.scenes.new('Fallen log land and water contact');bpy.context.window.scene=scene;objects=[]
    for o in old.all_objects:
        if o.type!='MESH' or o.get('asset_group')!=ASSET:continue
        copy=o.copy();copy.parent=None;copy.matrix_world=o.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy);objects.append(copy)
    bed_z=-50.0
    def material(name,color,alpha=1):
        m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;l=m.node_tree.links;bs=n.get('Principled BSDF');bs.inputs['Base Color'].default_value=(*color,1);bs.inputs['Roughness'].default_value=.9
        if alpha<1:
            transparent=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');mix.inputs[0].default_value=alpha;l.new(transparent.outputs[0],mix.inputs[1]);l.new(bs.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],n.get('Material Output').inputs['Surface'])
        return m
    soil=material('Inferred neutral bank',(.27,.24,.17));bed=material('Unobserved riverbed',(.14,.12,.08));water=material('Inferred water level',(.11,.20,.15),.62)
    def mesh(name,vertices,faces,mat):
        data=bpy.data.meshes.new(name);data.from_pydata([tuple(v) for v in vertices],[],faces);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(data);bm.free();o=bpy.data.objects.new(name,data);scene.collection.objects.link(o);data.materials.append(mat);objects.append(o);return o
    bank_faces=[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)]
    left=world((582,918),0);top=[left+Vector((x,y,0)) for x,y in [(-65,-50),(14,-50),(14,50),(-65,50)]]
    bottom=[Vector((p.x,p.y,bed_z)) for p in top]
    mesh('Provisional west bank',top+bottom,bank_faces,soil)
    rock_center=world((765,872),-7);vertices=[];faces=[];segments=24;rings=12
    vertices.append(tuple(rock_center+Vector((0,0,16))))
    for i in range(1,rings):
        phi=math.pi*i/rings
        for j in range(segments):
            theta=math.tau*j/segments;vertices.append(tuple(rock_center+Vector((20*math.sin(phi)*math.cos(theta),20*math.sin(phi)*math.sin(theta),(16 if math.cos(phi)>=0 else 43.5)*math.cos(phi)))))
    vertices.append(tuple(rock_center-Vector((0,0,43.5))))
    for j in range(segments):faces.append((0,1+j,1+(j+1)%segments))
    for i in range(rings-2):
        for j in range(segments):faces.append((1+i*segments+j,1+i*segments+(j+1)%segments,1+(i+1)*segments+(j+1)%segments,1+(i+1)*segments+j))
    last=1+(rings-2)*segments
    for j in range(segments):faces.append((len(vertices)-1,last+(j+1)%segments,last+j))
    stone=material('Inferred unrefined support stone',(.32,.29,.24));mesh('Source-sized provisional east support rock',vertices,faces,stone)
    water_corners=[world(p,-35) for p in [(590,875),(798,840),(798,950),(590,965)]]
    mesh('Water below fallen trunk',water_corners,[(0,1,2,3)],water)
    mesh('Provisional riverbed',[Vector((p.x,p.y,bed_z)) for p in water_corners],[(0,1,2,3)],bed)
    sun=bpy.data.lights.new('Contact inspection sun','SUN');sun.energy=2;light=bpy.data.objects.new(sun.name,sun);scene.collection.objects.link(light);light.rotation_euler=Vector((-.45,-.55,.70)).to_track_quat('Z','Y').to_euler();scene.world=bpy.data.worlds.new('Contact inspection world');scene.world.color=(.18,.18,.18)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.cycles.use_denoising=False;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    packet=json.loads((worker/'modified/views.json').read_text());points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];views={}
    for v in packet['views']:
        data=bpy.data.cameras.new('Fallen log contact '+str(v['index']));data.type='ORTHO';data.ortho_scale=v['ortho_scale']*1.65;inverse=Matrix(v['camera_matrix_world']).inverted();data.clip_start,data.clip_end=depth_clip_range(-(inverse@p).z for p in points);camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.matrix_world=Matrix(v['camera_matrix_world']);views[f'view-{v["index"]}']=camera.name
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'joint.blend'))
    render_views(scene.name,views,out,modes=('textured',),width=384)
    images=[Image.open(out/f'view-{i}-textured.png').convert('RGB') for i in range(8)];tw,th=images[0].size;sheet=Image.new('RGB',(tw*4,th*2))
    for i,im in enumerate(images):sheet.paste(im,((i%4)*tw,(i//4)*th))
    sheet.save(out/'sheet.png');write_json(out/'evidence.json',dict(model_sha256=digest,joint_sha256=sha(out/'joint.blend'),sheet_sha256=sha(out/'sheet.png'),west_bank_top_z=0,water_z=-35,bed_z=bed_z,east_rock_center=list(rock_center),east_rock_upper_radii=[20,20,16],east_rock_lower_height=43.5,rock_bed_penetration=.5,limitations=['Private contact hypothesis only, not finished bank or stone reconstruction.','Native artwork gives support rock extent approximately x745..785,y850..892; ellipsoid thickness and source-ray placement are inferred.','No obstacle or elevation data constrains the unmasked support stone or trunk.','Rock extends through the inferred water to bedZ-50 with0.5 penetration; concealed lower volume is inferred, not a floating cropped boulder.','West trunk is seated at landZ0; east curved end overlaps the rock surface as a provisional contact. Native-source occlusion and all8 must be inspected before accepting this placement.','WaterZ-35 and bedZ-50 are inferred and cannot be called measured native depths.','Final riverbank source surfaces, concealed vegetation and hidden texture remain unfinished.']))
    if sha(worker/'model.blend')!=digest:raise ValueError('Private worker changed during contact proof')
    release()

if __name__=='__main__':main()
