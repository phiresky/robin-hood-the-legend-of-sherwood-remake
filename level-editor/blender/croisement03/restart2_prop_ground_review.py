"""Actual saved prop against a diagnostic ground plane, with native camera first."""
import argparse,json,sys,hashlib,itertools
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from texture_camera import depth_clip_range
from evidence_io import sha,write_json

def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);parser.add_argument('--output-subdir',default='ground-contact');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);w=args.workspace.resolve();out=w/'inspection'/args.output_subdir;out.mkdir(exist_ok=False);digest=sha(w/'model.blend');acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));scene=bpy.data.scenes.new('Prop actual ground contact');objects=[]
        for obj in tuple(bpy.data.collections['Croisement03 Working'].all_objects):
            if obj.type=='MESH' and obj.get('asset_group')==w.name:
                world=obj.matrix_world.copy();copy=obj.copy();scene.collection.objects.link(copy);copy.parent=None;copy.matrix_world=world;copy.hide_render=False;objects.append(copy)
        scene.view_layers[0].update()
        write_json(out/'transform-audit.json',[dict(name=o.name,matrix=[list(row) for row in o.matrix_world],minimum_world_z=min((o.matrix_world@v.co).z for v in o.data.vertices)) for o in objects])
        assert objects;points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];ground_tolerance=max(1e-4,max(abs(c) for p in points for c in p)*2**-20);ground_points=[p for p in points if abs(p.z)<ground_tolerance];assert min(p.z for p in points)>=-ground_tolerance
        area=max(((b-a).cross(c-a).length/2 for a,b,c in itertools.combinations(ground_points,3)),default=0);assert area>1.,('Insufficient stable ground patch',len(ground_points),area)
        x0,x1=min(p.x for p in points)-15,max(p.x for p in points)+15;y0,y1=min(p.y for p in points)-15,max(p.y for p in points)+15
        mesh=bpy.data.meshes.new('Diagnostic zero-height ground');mesh.from_pydata([(x0,y0,0),(x1,y0,0),(x1,y1,0),(x0,y1,0)],[],[(0,1,2,3)]);mesh.update();ground=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(ground)
        mat=bpy.data.materials.new('Neutral diagnostic ground');mat.use_nodes=True;mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.22,.25,.19,1);mat.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1;mesh.materials.append(mat)
        sun=bpy.data.lights.new('Ground review sun','SUN');sun.energy=2;light=bpy.data.objects.new(sun.name,sun);scene.collection.objects.link(light);light.rotation_euler=Vector((-.45,-.55,.70)).to_track_quat('Z','Y').to_euler();scene.world=bpy.data.worlds.new('Ground review world');scene.world.color=(.18,.18,.18);scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        packet=json.loads((w/'modified/views.json').read_text());all_points=points+[Vector(v) for v in [(x0,y0,0),(x1,y0,0),(x1,y1,0),(x0,y1,0)]];views={}
        for v in packet['views']:
            camera=bpy.data.cameras.new('Ground view '+str(v['index']));camera.type='ORTHO';camera.ortho_scale=v['ortho_scale']*1.12;inverse=Matrix(v['camera_matrix_world']).inverted();camera.clip_start,camera.clip_end=depth_clip_range(-(inverse@p).z for p in all_points);obj=bpy.data.objects.new(camera.name,camera);scene.collection.objects.link(obj);obj.matrix_world=Matrix(v['camera_matrix_world']);views[f'view-{v["index"]}']=obj.name
        render_views(scene.name,views,out,modes=('textured',),width=384);images=[Image.open(out/f'view-{i}-textured.png').convert('RGB') for i in range(8)];tw,th=images[0].size;sheet=Image.new('RGB',(tw*4,th*2))
        for i,im in enumerate(images):sheet.paste(im,((i%4)*tw,(i//4)*th))
        sheet.save(out/'sheet.png');write_json(out/'evidence.json',dict(model_sha256=digest,sheet_sha256=sha(out/'sheet.png'),native_view_index=0,ground_z=0,ground_tolerance=ground_tolerance,tolerance_basis='Eight float32 coordinate ulps bound world-transform roundoff; contact must still span a nonzero triangle.',minimum_z=min(p.z for p in points),ground_contact_vertices=len(ground_points),maximum_contact_triangle_area=area,limitations=['Diagnostic ground only. Foreground shrub and neighboring terrain/asset integration remain unfinished.','This check confirms physical seating, not native foreground ownership or final map completion.']));assert sha(w/'model.blend')==digest;print(out/'sheet.png')
    finally:release()
if __name__=='__main__':main()
